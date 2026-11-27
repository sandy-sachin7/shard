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

struct TwoCommitFixture {
    tmp: TempDir,
    head_v1: String,
    head_v2: String,
    a_v1: Vec<u8>,
    b_v1: Vec<u8>,
    a_v2: Vec<u8>,
    b_v2: Vec<u8>,
    c_v2: Vec<u8>,
}

fn setup_two_commit_repo() -> TwoCommitFixture {
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

    let a_v1 = b"alpha version 1 original bytes".to_vec();
    let b_v1 = b"beta version 1 original bytes".to_vec();
    fs::write(repo.join("a.bin"), &a_v1).unwrap();
    fs::write(repo.join("b.bin"), &b_v1).unwrap();
    shard_core::add(repo, &repo.join("a.bin"), true).unwrap();
    shard_core::add(repo, &repo.join("b.bin"), true).unwrap();
    shard_core::commit(repo, "v1", "Test <t@local>", true).unwrap();

    let head_v1 = fs::read_to_string(repo.join(".shard/refs/heads/main"))
        .unwrap()
        .trim()
        .to_string();

    let a_v2 = b"alpha version 2 updated bytes".to_vec();
    let b_v2 = b"beta version 2 updated bytes".to_vec();
    let c_v2 = b"gamma version 2 newly introduced file".to_vec();
    fs::write(repo.join("a.bin"), &a_v2).unwrap();
    fs::write(repo.join("b.bin"), &b_v2).unwrap();
    fs::write(repo.join("c.bin"), &c_v2).unwrap();
    shard_core::add(repo, &repo.join("a.bin"), true).unwrap();
    shard_core::add(repo, &repo.join("b.bin"), true).unwrap();
    shard_core::add(repo, &repo.join("c.bin"), true).unwrap();
    shard_core::commit(repo, "v2", "Test <t@local>", true).unwrap();

    let head_v2 = fs::read_to_string(repo.join(".shard/refs/heads/main"))
        .unwrap()
        .trim()
        .to_string();

    // Return to v1 and remove c.bin so v1 working tree only has a.bin and b.bin.
    shard_core::checkout(repo, &head_v1, true).unwrap();
    let _ = fs::remove_file(repo.join("c.bin"));
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), a_v1);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), b_v1);
    assert!(!repo.join("c.bin").exists());

    TwoCommitFixture {
        tmp,
        head_v1,
        head_v2,
        a_v1,
        b_v1,
        a_v2,
        b_v2,
        c_v2,
    }
}

#[test]
fn test_checkout_fault_during_backup_phase_touches_nothing() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let err = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::BackupAt(1)),
    );
    assert!(err.is_err());
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v1);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v1);
    assert!(!repo.join("c.bin").exists());
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));
}

#[test]
fn test_checkout_fault_mid_swap_rolls_back_earlier_overwritten_files() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    // Index 0 is a.bin (swapped into place), index 1 is b.bin (injected swap failure).
    let err = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::SwapAt(1)),
    );
    assert!(err.is_err());
    assert_eq!(
        fs::read(repo.join("a.bin")).unwrap(),
        f.a_v1,
        "a.bin was already swapped and MUST be rolled back to v1"
    );
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v1);
    assert!(!repo.join("c.bin").exists());
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));
}

#[test]
fn test_checkout_fault_on_head_update_rolls_back_all_swapped_files() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let err = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::HeadUpdate),
    );
    assert!(err.is_err());
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v1);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v1);
    assert!(
        !repo.join("c.bin").exists(),
        "newly created file c.bin must be removed on HEAD-update rollback"
    );
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));
}

#[test]
fn test_checkout_crash_recovery_rolls_back_mid_swap_journal() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    // Simulate process crash after a.bin was replaced (index 0) and before b.bin (index 1).
    let err = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::CrashMidSwap(1)),
    );
    assert!(err.is_err());
    assert_eq!(
        fs::read(repo.join("a.bin")).unwrap(),
        f.a_v2,
        "precondition: crash left a.bin at v2"
    );
    assert_eq!(
        fs::read(repo.join("b.bin")).unwrap(),
        f.b_v1,
        "precondition: crash left b.bin at v1"
    );
    assert!(repo.join(".shard/checkout_journal.json").exists());

    // Running `shard recover` must deterministically roll back to v1 and remove the journal.
    shard_core::recover(repo, true).unwrap();
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v1);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v1);
    assert!(!repo.join("c.bin").exists());
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));
}

#[test]
fn test_checkout_crash_recovery_rolls_forward_at_updating_head_phase() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    // Simulate crash after all files are swapped + fsynced, journal at UpdatingHead, HEAD still v1.
    let err = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::CrashBeforeHeadUpdate),
    );
    assert!(err.is_err());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));
    assert!(repo.join(".shard/checkout_journal.json").exists());

    // Recovery rolls forward HEAD to v2 and cleans up journal + backups.
    shard_core::recover(repo, true).unwrap();
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v2);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v2);
    assert_eq!(fs::read(repo.join("c.bin")).unwrap(), f.c_v2);
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v2));
}

#[test]
fn test_checkout_surfaces_rollback_errors_and_preserves_journal() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let err = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::CorruptBackupBeforeSwapFail(1)),
    )
    .expect_err("must report rollback failure");
    let msg = err.to_string();
    assert!(
        msg.contains("rollback also encountered errors"),
        "expected explicit rollback error message, got: {msg}"
    );
    assert!(
        repo.join(".shard/checkout_journal.json").exists(),
        "journal must remain on disk when rollback fails so manual recovery can retry"
    );
}

#[test]
fn test_checkout_journal_crc32_corruption_fails_loudly() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let _ = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::CrashMidSwap(1)),
    );
    let jp = repo.join(".shard/checkout_journal.json");
    assert!(jp.exists());

    // Tamper with a single byte inside the serialized payload so CRC32 fails.
    let mut text = fs::read_to_string(&jp).unwrap();
    text = text.replacen("Swapping", "SwappinX", 1);
    fs::write(&jp, text).unwrap();

    let err = shard_core::recover(repo, true).expect_err("corrupted journal must fail loudly");
    let msg = err.to_string();
    assert!(
        msg.contains("CRC32 mismatch"),
        "expected CRC32 mismatch error, got: {msg}"
    );
}

#[test]
fn test_checkout_backup_bitrot_detected_by_blake3_during_recovery() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let _ = shard_core::checkout_with_fault(
        repo,
        &f.head_v2,
        true,
        Some(shard_core::CheckoutFault::CrashMidSwap(1)),
    );
    assert!(repo.join(".shard/checkout_journal.json").exists());

    // Locate the durable backup for a.bin and flip 1 byte to simulate silent disk corruption.
    let mut corrupted = false;
    for entry in fs::read_dir(repo).unwrap() {
        let p = entry.unwrap().path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.starts_with(".a.bin.shardbak.") {
                let mut bytes = fs::read(&p).unwrap();
                bytes[0] ^= 0xff;
                fs::write(&p, bytes).unwrap();
                corrupted = true;
            }
        }
    }
    assert!(corrupted, "expected .a.bin.shardbak.* to exist");

    let err = shard_core::recover(repo, true)
        .expect_err("bit-rot in backup must be caught by Blake3 check");
    let msg = err.to_string();
    assert!(
        msg.contains("backup integrity mismatch"),
        "expected backup integrity mismatch error, got: {msg}"
    );
    assert!(repo.join(".shard/checkout_journal.json").exists());
}

#[test]
#[ignore = "helper subprocess invoked by real hard-kill crash recovery tests"]
fn child_hard_crash_checkout() {
    let repo = std::path::PathBuf::from(std::env::var("SHARD_CHILD_REPO").unwrap());
    let target = std::env::var("SHARD_CHILD_TARGET").unwrap();
    let stage = std::env::var("SHARD_CHILD_CRASH").unwrap();
    let fault = match stage.as_str() {
        "mid_swap" => shard_core::CheckoutFault::HardCrashMidSwap(1),
        "before_head" => shard_core::CheckoutFault::HardCrashBeforeHeadUpdate,
        other => panic!("unexpected SHARD_CHILD_CRASH={other}"),
    };
    let _ = shard_core::checkout_with_fault(&repo, &target, true, Some(fault));
}

#[test]
fn test_checkout_real_process_hard_kill_mid_swap_recovers_cleanly() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_hard_crash_checkout", "--ignored"])
        .env("SHARD_CHILD_REPO", repo)
        .env("SHARD_CHILD_TARGET", &f.head_v2)
        .env("SHARD_CHILD_CRASH", "mid_swap")
        .status()
        .expect("failed to spawn child process");
    assert_eq!(status.code(), Some(137), "child must hard-exit with 137");

    // Process died mid-swap without running any Drop destructors.
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v2);
    assert!(repo.join(".shard/checkout_journal.json").exists());

    // A fresh process recovers cleanly back to v1.
    shard_core::recover(repo, true).unwrap();
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v1);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v1);
    assert!(!repo.join("c.bin").exists());
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));
}

#[test]
fn test_checkout_real_process_hard_kill_before_head_rolls_forward() {
    let f = setup_two_commit_repo();
    let repo = f.tmp.path();

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_hard_crash_checkout", "--ignored"])
        .env("SHARD_CHILD_REPO", repo)
        .env("SHARD_CHILD_TARGET", &f.head_v2)
        .env("SHARD_CHILD_CRASH", "before_head")
        .status()
        .expect("failed to spawn child process");
    assert_eq!(status.code(), Some(137), "child must hard-exit with 137");

    assert!(repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v1));

    // Fresh process rolls forward cleanly to v2.
    shard_core::recover(repo, true).unwrap();
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), f.a_v2);
    assert_eq!(fs::read(repo.join("b.bin")).unwrap(), f.b_v2);
    assert_eq!(fs::read(repo.join("c.bin")).unwrap(), f.c_v2);
    assert!(!repo.join(".shard/checkout_journal.json").exists());
    assert!(fs::read_to_string(repo.join(".shard/HEAD"))
        .unwrap()
        .contains(&f.head_v2));
}

#[cfg(unix)]
#[test]
fn test_pinned_repo_dir_and_checkout_reject_symlinks() {
    use std::os::unix::fs::symlink;

    let f = setup_two_commit_repo();
    let repo = f.tmp.path();
    let outside = TempDir::new().unwrap();
    let outside_secret = outside.path().join("secret.txt");
    fs::write(&outside_secret, b"DO NOT OVERWRITE").unwrap();

    // 1. Direct PinnedRepoDir symlink refusal (leaf symlink + directory symlink).
    let pinned = shard_core::fs_util::PinnedRepoDir::open(repo).unwrap();
    symlink(&outside_secret, repo.join("link_leaf.bin")).unwrap();
    assert!(pinned.exists_no_follow("link_leaf.bin").is_err());
    assert!(pinned.open_read("link_leaf.bin").is_err());

    symlink(outside.path(), repo.join("link_dir")).unwrap();
    assert!(pinned.exists_no_follow("link_dir/secret.txt").is_err());
    assert!(pinned.open_read("link_dir/secret.txt").is_err());

    // 2. Replace a.bin with a symlink pointing outside the repo; checkout must refuse it
    //    via O_NOFOLLOW / AT_SYMLINK_NOFOLLOW and leave the external file untouched.
    fs::remove_file(repo.join("a.bin")).unwrap();
    symlink(&outside_secret, repo.join("a.bin")).unwrap();
    let err = shard_core::checkout(repo, &f.head_v2, true)
        .expect_err("checkout must reject symlink destination");
    assert!(
        err.to_string().contains("symlink"),
        "expected symlink rejection, got: {err}"
    );
    assert_eq!(
        fs::read(&outside_secret).unwrap(),
        b"DO NOT OVERWRITE",
        "external symlink target must never be modified"
    );
}
