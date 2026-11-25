use shard_core::chunk_codec;
use shard_core::chunker::{Chunker, ChunkerMode};
use shard_core::compression::Compression;
use shard_core::encryption::{generate_repo_key, RepoCipher};
use shard_core::wal::{Wal, WalEntry};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn pseudo_random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut s = seed;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        out.push((xorshift64(&mut s) & 0xFF) as u8);
    }
    out
}

#[test]
fn test_fastcdc_property_roundtrip_and_shift_invariance() {
    let configs = [
        (512u64, 2048u64, 8192u64),
        (1024, 4096, 16384),
        (4096, 16384, 65536),
    ];

    for (idx, &(min, avg, max)) in configs.iter().enumerate() {
        let base = pseudo_random_bytes(0xA1B2_C3D4_0000_0001 + idx as u64, 256 * 1024);

        let mut c1 = Chunker::new_fastcdc(Box::new(Cursor::new(base.clone())), min, avg, max);
        let mut reconstructed = Vec::new();
        let mut hashes1 = HashSet::new();
        let mut prev_offset = 0u64;
        while let Some(chunk) = c1.next_chunk().unwrap() {
            assert_eq!(chunk.offset, prev_offset);
            assert!(chunk.data.len() <= max as usize);
            prev_offset += chunk.data.len() as u64;
            hashes1.insert(blake3::hash(&chunk.data));
            reconstructed.extend_from_slice(&chunk.data);
        }
        assert_eq!(reconstructed, base, "FastCDC roundtrip must be byte-exact");

        // Prepend random bytes and verify strong chunk reuse after boundary resynchronization.
        let mut shifted = pseudo_random_bytes(0x9999_8888_7777_6666 + idx as u64, 1500);
        shifted.extend_from_slice(&base);

        let mut c2 = Chunker::new_fastcdc(Box::new(Cursor::new(shifted.clone())), min, avg, max);
        let mut recon2 = Vec::new();
        let mut reused = 0usize;
        let mut total = 0usize;
        while let Some(chunk) = c2.next_chunk().unwrap() {
            total += 1;
            if hashes1.contains(&blake3::hash(&chunk.data)) {
                reused += 1;
            }
            recon2.extend_from_slice(&chunk.data);
        }
        assert_eq!(recon2, shifted);
        let reuse_ratio = reused as f64 / total as f64;
        assert!(
            reuse_ratio >= 0.75,
            "expected >=75% shift-invariant chunk reuse for ({min},{avg},{max}), got {:.1}%",
            reuse_ratio * 100.0
        );
    }
}

#[test]
fn test_shrd_envelope_rejects_corruption_truncation_and_aad_swap() {
    let key = generate_repo_key();
    let cipher = RepoCipher::from_key(&key);
    let plaintext = b"trained tensor payload bytes for SHRD envelope verification";
    let chunk_id = blake3::hash(plaintext).to_hex().to_string();

    let encoded =
        chunk_codec::encode_chunk(&chunk_id, plaintext, Compression::Zstd, Some(&cipher)).unwrap();
    let decoded =
        chunk_codec::decode_chunk(&chunk_id, &encoded, Compression::Zstd, Some(&cipher)).unwrap();
    assert_eq!(decoded, plaintext);

    // 1. Cross-chunk AAD swap must fail authentication.
    let other_id = blake3::hash(b"different chunk").to_hex().to_string();
    assert!(
        chunk_codec::decode_chunk(&other_id, &encoded, Compression::Zstd, Some(&cipher)).is_err()
    );

    // 2. Single-bit flip anywhere in the envelope must fail cleanly without panic.
    for pos in [4, 5, 8, encoded.len() / 2, encoded.len() - 1] {
        let mut mutated = encoded.clone();
        mutated[pos] ^= 0x01;
        assert!(
            chunk_codec::decode_chunk(&chunk_id, &mutated, Compression::Zstd, Some(&cipher))
                .is_err(),
            "bit flip at offset {pos} must be rejected"
        );
    }

    // 3. Truncated envelopes and garbage bytes never panic.
    for trunc_len in [0, 1, 4, 7, 8, 12, encoded.len() - 1] {
        assert!(chunk_codec::decode_chunk(
            &chunk_id,
            &encoded[..trunc_len],
            Compression::Zstd,
            Some(&cipher)
        )
        .is_err());
    }
    let mut seed = 0xCAFE_BABE_1234_5678u64;
    for _ in 0..200 {
        let len = (xorshift64(&mut seed) % 256) as usize;
        let junk = pseudo_random_bytes(xorshift64(&mut seed), len);
        let _ = chunk_codec::decode_chunk(&chunk_id, &junk, Compression::Zstd, Some(&cipher));
    }
}

#[test]
fn test_wal_truncated_and_corrupted_crc_recovery_does_not_panic() {
    let tmp = TempDir::new().unwrap();
    let shard_dir = tmp.path().join(".shard");
    fs::create_dir_all(&shard_dir).unwrap();
    let wal = Wal::new(&shard_dir);

    wal.append(&WalEntry::AddBegin {
        path: "model.safetensors".to_string(),
    })
    .unwrap();
    wal.append(&WalEntry::AddEnd).unwrap();

    // Append a torn trailing record (simulating kill -9 mid-write).
    let wal_path = shard_dir.join("wal.log");
    let mut raw = fs::read_to_string(&wal_path).unwrap();
    raw.push_str("deadbeef|{\"CommitBegin\":{\"head_backup\":");
    fs::write(&wal_path, raw).unwrap();

    let recovered = wal.read().unwrap();
    assert_eq!(
        recovered.len(),
        2,
        "valid prefix records must survive torn tail"
    );

    // Corrupt a middle record's CRC32 checksum: replay stops cleanly without panicking.
    fs::write(
        &wal_path,
        "00000000|{\"AddBegin\":{\"path\":\"tampered\"}}\n",
    )
    .unwrap();
    let entries = wal.read().unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_checkout_failure_is_atomic_across_multiple_files() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();

    shard_core::init_with_passphrase(
        repo,
        "flat",
        "zstd",
        "fastcdc",
        Some(65_536),
        false,
        true,
        "",
    )
    .unwrap();

    // Commit v1 with two files.
    let a_v1 = b"file A version 1 original content".to_vec();
    let b_v1 = b"file B version 1 original content".to_vec();
    fs::write(repo.join("a.bin"), &a_v1).unwrap();
    fs::write(repo.join("b.bin"), &b_v1).unwrap();
    shard_core::add(repo, &repo.join("a.bin"), true).unwrap();
    shard_core::add(repo, &repo.join("b.bin"), true).unwrap();
    shard_core::commit(repo, "v1", "Test <t@local>", true).unwrap();

    let head_v1 = fs::read_to_string(repo.join(".shard/refs/heads/main"))
        .unwrap()
        .trim()
        .to_string();

    // Commit v2 modifying both files.
    let a_v2 = b"file A version 2 updated content".to_vec();
    let b_v2 = b"file B version 2 updated content that will be corrupted".to_vec();
    fs::write(repo.join("a.bin"), &a_v2).unwrap();
    fs::write(repo.join("b.bin"), &b_v2).unwrap();
    shard_core::add(repo, &repo.join("a.bin"), true).unwrap();
    shard_core::add(repo, &repo.join("b.bin"), true).unwrap();
    shard_core::commit(repo, "v2", "Test <t@local>", true).unwrap();

    let head_v2 = fs::read_to_string(repo.join(".shard/refs/heads/main"))
        .unwrap()
        .trim()
        .to_string();

    // Switch back to v1 cleanly.
    shard_core::checkout(repo, &head_v1, true).unwrap();
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), a_v1);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), b_v1);

    // Corrupt b_v2's chunk in FlatStore so checking out v2 fails on the second file.
    let b_v2_chunk_id = blake3::hash(&b_v2).to_hex().to_string();
    let obj_path = repo
        .join(".shard/objects")
        .join(&b_v2_chunk_id[..2])
        .join(&b_v2_chunk_id);
    assert!(obj_path.exists(), "expected flat object at {obj_path:?}");
    let mut corrupted = fs::read(&obj_path).unwrap();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xFF;
    fs::write(&obj_path, corrupted).unwrap();

    // Checkout of v2 must fail AND leave a.bin untouched at v1 + HEAD untouched at v1.
    let err = shard_core::checkout(repo, &head_v2, true);
    assert!(err.is_err(), "corrupted v2 checkout must fail");
    assert_eq!(
        fs::read(repo.join("a.bin")).unwrap(),
        a_v1,
        "a.bin must not be overwritten when b.bin fails verification during multi-file checkout"
    );
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), b_v1);
    let head_after = fs::read_to_string(repo.join(".shard/HEAD")).unwrap();
    assert!(
        head_after.contains(&head_v1),
        "HEAD must stay on v1 after failed checkout, got: {head_after}"
    );
}

#[test]
fn test_fastcdc_config_normalization_and_rabin_alias() {
    let tmp = TempDir::new().unwrap();
    shard_core::init_with_passphrase(
        tmp.path(),
        "flat",
        "zstd",
        "rabin",
        Some(65_536),
        false,
        true,
        "",
    )
    .unwrap();
    let cfg = shard_core::config::load_config(&tmp.path().join(".shard")).unwrap();
    assert_eq!(cfg.get("chunker_mode").map(String::as_str), Some("fastcdc"));
    assert!(matches!(
        ChunkerMode::from_config(&cfg),
        ChunkerMode::FastCdc { .. }
    ));
}
