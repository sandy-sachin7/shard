use crate::compression::Compression;
use crate::encryption::RepoCipher;
use crate::error::ShardError;
use crate::fs_util::validate_object_id;
use anyhow::Result;

/// 8-byte self-describing chunk envelope header:
/// `[0..4] = b"SHRD"`, `[4] = version (1)`, `[5] = flags`, `[6..8] = reserved (0, 0)`.
///
/// Flags layout:
/// - bits 0..2: compression algorithm (`0 = none`, `1 = zstd`, `2 = zlib`)
/// - bit 2 (`0x04`): encrypted with AES-256-GCM (AAD = chunk_id UTF-8 bytes)
const ENVELOPE_MAGIC: &[u8; 4] = b"SHRD";
const ENVELOPE_VERSION: u8 = 1;
const ENVELOPE_HEADER_LEN: usize = 8;
const FLAG_COMPRESSION_MASK: u8 = 0x03;
const FLAG_ENCRYPTED: u8 = 0x04;

/// Parsed header of a self-describing `SHRD` v1 chunk envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    pub compression: Compression,
    pub encrypted: bool,
}

/// Inspect whether `stored` begins with a valid `SHRD` v1 envelope header.
pub fn parse_header(stored: &[u8]) -> Option<ChunkHeader> {
    if stored.len() < ENVELOPE_HEADER_LEN
        || &stored[..4] != ENVELOPE_MAGIC
        || stored[4] != ENVELOPE_VERSION
        || stored[6] != 0
        || stored[7] != 0
    {
        return None;
    }
    let flags = stored[5];
    let compression = Compression::from_code(flags & FLAG_COMPRESSION_MASK).ok()?;
    let encrypted = (flags & FLAG_ENCRYPTED) != 0;
    Some(ChunkHeader {
        compression,
        encrypted,
    })
}

/// Encode plaintext chunk bytes into a self-describing `SHRD` v1 chunk envelope.
/// When `cipher` is `Some`, encrypts the compressed payload with AAD bound to `chunk_id`.
pub fn encode_chunk(
    chunk_id: &str,
    plaintext: &[u8],
    compression: Compression,
    cipher: Option<&RepoCipher>,
) -> Result<Vec<u8>> {
    validate_object_id(chunk_id)?;
    let compressed = compression.compress(plaintext)?;
    let (payload, encrypted_flag) = match cipher {
        Some(c) => (
            c.encrypt_with_aad(&compressed, chunk_id.as_bytes())?,
            FLAG_ENCRYPTED,
        ),
        None => (compressed, 0u8),
    };
    let flags = (compression.code() & FLAG_COMPRESSION_MASK) | encrypted_flag;
    let mut out = Vec::with_capacity(ENVELOPE_HEADER_LEN + payload.len());
    out.extend_from_slice(ENVELOPE_MAGIC);
    out.push(ENVELOPE_VERSION);
    out.push(flags);
    out.extend_from_slice(&[0u8, 0u8]);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode stored chunk bytes and verify that `blake3(plaintext) == chunk_id`.
///
/// Supports both `SHRD` v1 self-describing envelopes (with AAD-bound encryption)
/// and legacy headerless blobs (using `fallback_compression` and non-AAD decryption).
pub fn decode_chunk(
    chunk_id: &str,
    stored: &[u8],
    fallback_compression: Compression,
    cipher: Option<&RepoCipher>,
) -> Result<Vec<u8>> {
    validate_object_id(chunk_id)?;

    let plaintext = if let Some(header) = parse_header(stored) {
        let payload = &stored[ENVELOPE_HEADER_LEN..];
        let decrypted = if header.encrypted {
            let c = cipher.ok_or_else(|| {
                anyhow::anyhow!(
                    "chunk '{}' is encrypted but repository cipher is unavailable",
                    chunk_id
                )
            })?;
            c.decrypt_with_aad(payload, chunk_id.as_bytes())?
        } else {
            payload.to_vec()
        };
        header.compression.decompress(&decrypted)?
    } else {
        // Legacy format without SHRD envelope header
        let decrypted = match cipher {
            Some(c) => c
                .decrypt_with_aad(stored, chunk_id.as_bytes())
                .or_else(|_| c.decrypt(stored))?,
            None => stored.to_vec(),
        };
        fallback_compression.decompress(&decrypted)?
    };

    let actual = blake3::hash(&plaintext).to_hex().to_string();
    if actual != chunk_id {
        return Err(ShardError::HashMismatch {
            kind: "chunk",
            id: chunk_id.to_string(),
            expected: chunk_id.to_string(),
            actual,
        }
        .into());
    }
    Ok(plaintext)
}

/// Verify an arbitrary object received over P2P (`PutChunk`) before storing it.
///
/// Accepts:
/// 1. Raw content-addressed metadata objects (commits, manifests, key rotations, legacy uncompressed
///    chunks) where `blake3(data) == id`.
/// 2. Self-describing `SHRD` v1 chunks whose decrypted + decompressed plaintext hashes to `id`.
pub fn verify_wire_object(id: &str, data: &[u8], cipher: Option<&RepoCipher>) -> bool {
    if validate_object_id(id).is_err() {
        return false;
    }
    if blake3::hash(data).to_hex().as_str() == id {
        return true;
    }
    if parse_header(data).is_some() {
        return decode_chunk(id, data, Compression::None, cipher).is_ok();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_all_modes() {
        let data = b"hello shard self-describing chunk envelope!".repeat(64);
        let id = blake3::hash(&data).to_hex().to_string();
        let cipher = RepoCipher::generate();

        for comp in [Compression::None, Compression::Zstd, Compression::Zlib] {
            let plain_env = encode_chunk(&id, &data, comp, None).unwrap();
            assert_eq!(
                decode_chunk(&id, &plain_env, Compression::None, None).unwrap(),
                data
            );
            assert!(verify_wire_object(&id, &plain_env, None));

            let enc_env = encode_chunk(&id, &data, comp, Some(&cipher)).unwrap();
            assert_eq!(
                decode_chunk(&id, &enc_env, Compression::None, Some(&cipher)).unwrap(),
                data
            );
            assert!(verify_wire_object(&id, &enc_env, Some(&cipher)));
            // Wrong cipher or missing cipher must fail
            assert!(decode_chunk(&id, &enc_env, Compression::None, None).is_err());
            assert!(!verify_wire_object(&id, &enc_env, None));
        }
    }

    #[test]
    fn test_legacy_blob_backward_compat() {
        let data = b"legacy zstd chunk payload";
        let id = blake3::hash(data).to_hex().to_string();
        let legacy_compressed = Compression::Zstd.compress(data).unwrap();
        let decoded = decode_chunk(&id, &legacy_compressed, Compression::Zstd, None).unwrap();
        assert_eq!(decoded, data);

        let cipher = RepoCipher::generate();
        let legacy_enc = cipher.encrypt(&legacy_compressed);
        let decoded_enc = decode_chunk(&id, &legacy_enc, Compression::Zstd, Some(&cipher)).unwrap();
        assert_eq!(decoded_enc, data);
    }

    #[test]
    fn test_aad_chunk_swap_rejected() {
        let cipher = RepoCipher::generate();
        let data = b"chunk A";
        let id_a = blake3::hash(data).to_hex().to_string();
        let id_b = blake3::hash(b"chunk B").to_hex().to_string();
        let enc_a = encode_chunk(&id_a, data, Compression::Zstd, Some(&cipher)).unwrap();
        assert!(decode_chunk(&id_b, &enc_a, Compression::Zstd, Some(&cipher)).is_err());
        assert!(!verify_wire_object(&id_b, &enc_a, Some(&cipher)));
    }
}
