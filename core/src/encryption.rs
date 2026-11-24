use aes_gcm::aead::{Aead, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::Result;
use rand::RngCore;
use std::fs;
use std::path::Path;
use zeroize::Zeroizing;

const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;
const ENCRYPTED_KEY_PREFIX: &str = "enc:";

pub struct RepoCipher {
    cipher: Aes256Gcm,
}

impl RepoCipher {
    pub fn generate() -> Self {
        let key = Aes256Gcm::generate_key(OsRng);
        Self {
            cipher: Aes256Gcm::new(&key),
        }
    }

    pub fn from_key(key_bytes: &[u8; KEY_SIZE]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(key_bytes);
        Self {
            cipher: Aes256Gcm::new(key),
        }
    }

    /// Encrypt plaintext with empty AAD (legacy format).
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        self.encrypt_with_aad(plaintext, b"")
            .expect("AES-256-GCM encryption failed")
    }

    /// Encrypt plaintext binding `aad` (e.g., chunk content hash) into the GCM authentication tag.
    pub fn encrypt_with_aad(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("chunk encryption failed"))?;
        let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Decrypt legacy ciphertext (empty AAD).
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.decrypt_with_aad(data, b"")
    }

    /// Decrypt ciphertext authenticating `aad`.
    pub fn decrypt_with_aad(&self, data: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_SIZE + 16 {
            anyhow::bail!(
                "encrypted data too short (need at least {} bytes for nonce + auth tag)",
                NONCE_SIZE + 16
            );
        }
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(
                nonce,
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| {
                anyhow::anyhow!("decryption failed — wrong key, AAD mismatch, or corrupted data")
            })?;
        Ok(plaintext)
    }
}

pub fn generate_repo_key() -> [u8; KEY_SIZE] {
    let mut key = [0u8; KEY_SIZE];
    OsRng.fill_bytes(&mut key);
    key
}

/// Save `repo.key` unencrypted with `0o600` permissions (backward compatible).
pub fn save_repo_key(keys_dir: &Path, key: &[u8; KEY_SIZE]) -> Result<()> {
    save_repo_key_with_passphrase(keys_dir, key, "")
}

/// Save `repo.key` atomically with `0o600` permissions.
/// When `passphrase` is non-empty, wraps the key with Argon2id + AES-256-GCM.
pub fn save_repo_key_with_passphrase(
    keys_dir: &Path,
    key: &[u8; KEY_SIZE],
    passphrase: &str,
) -> Result<()> {
    let encoded = if passphrase.is_empty() {
        Zeroizing::new(hex::encode(key))
    } else {
        let wrapped = shard_crypto::encrypt_secret_bytes(key, passphrase)?;
        Zeroizing::new(format!("{}{}", ENCRYPTED_KEY_PREFIX, hex::encode(wrapped)))
    };
    shard_crypto::write_secret_file(&keys_dir.join("repo.key"), encoded.as_bytes())?;
    Ok(())
}

/// Load `repo.key` without passphrase (succeeds only for unencrypted `repo.key`).
pub fn load_repo_key(keys_dir: &Path) -> Result<[u8; KEY_SIZE]> {
    load_repo_key_with_passphrase(keys_dir, "")
}

/// Load `repo.key`, decrypting with `passphrase` if wrapped.
pub fn load_repo_key_with_passphrase(keys_dir: &Path, passphrase: &str) -> Result<[u8; KEY_SIZE]> {
    let raw = Zeroizing::new(fs::read_to_string(keys_dir.join("repo.key"))?);
    let trimmed = raw.trim();
    let bytes = if let Some(enc_hex) = trimmed.strip_prefix(ENCRYPTED_KEY_PREFIX) {
        if passphrase.is_empty() {
            anyhow::bail!(
                "repo.key is encrypted but no passphrase provided. Set SHARD_PASSPHRASE or unlock the repository"
            );
        }
        let enc_bytes = hex::decode(enc_hex)?;
        shard_crypto::decrypt_secret_bytes(&enc_bytes, passphrase)?
    } else {
        Zeroizing::new(hex::decode(trimmed)?)
    };

    if bytes.len() != KEY_SIZE {
        anyhow::bail!(
            "invalid repo.key: expected {} bytes, got {}",
            KEY_SIZE,
            bytes.len()
        );
    }
    let mut key = [0u8; KEY_SIZE];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let cipher = RepoCipher::generate();
        let data = b"hello world this is test data";
        let encrypted = cipher.encrypt(data);
        assert_ne!(encrypted, data);
        assert!(encrypted.len() > NONCE_SIZE);
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_encrypt_decrypt_with_aad_binds_chunk_id() {
        let cipher = RepoCipher::generate();
        let data = b"model weights chunk";
        let aad = b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let encrypted = cipher.encrypt_with_aad(data, aad).unwrap();
        assert_eq!(cipher.decrypt_with_aad(&encrypted, aad).unwrap(), data);
        // Different AAD (chunk swap attack) must fail
        let wrong_aad = b"fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        assert!(cipher.decrypt_with_aad(&encrypted, wrong_aad).is_err());
    }

    #[test]
    fn test_encrypt_different_nonces() {
        let cipher = RepoCipher::generate();
        let data = b"same data";
        let e1 = cipher.encrypt(data);
        let e2 = cipher.encrypt(data);
        assert_ne!(
            e1, e2,
            "two encryptions of same data should differ (random nonce)"
        );
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let cipher1 = RepoCipher::generate();
        let cipher2 = RepoCipher::generate();
        let data = b"secret message";
        let encrypted = cipher1.encrypt(data);
        let result = cipher2.decrypt(&encrypted);
        assert!(result.is_err(), "decrypt with wrong key should fail");
    }

    #[test]
    fn test_decrypt_tampered() {
        let cipher = RepoCipher::generate();
        let data = b"tamper me";
        let mut encrypted = cipher.encrypt(data);
        if encrypted.len() > NONCE_SIZE + 1 {
            encrypted[NONCE_SIZE] ^= 0xFF;
        }
        let result = cipher.decrypt(&encrypted);
        assert!(
            result.is_err(),
            "decrypt of tampered ciphertext should fail"
        );
    }

    #[test]
    fn test_short_data() {
        let cipher = RepoCipher::generate();
        let result = cipher.decrypt(&[0u8; 5]);
        assert!(
            result.is_err(),
            "decrypt of data shorter than nonce should fail"
        );
    }

    #[test]
    fn test_key_save_load_roundtrip() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let key = generate_repo_key();
        save_repo_key(dir.path(), &key).unwrap();
        let loaded = load_repo_key(dir.path()).unwrap();
        assert_eq!(key, loaded);
    }

    #[test]
    fn test_key_save_load_with_passphrase_roundtrip() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let key = generate_repo_key();
        save_repo_key_with_passphrase(dir.path(), &key, "strong-pass").unwrap();
        assert!(load_repo_key(dir.path()).is_err());
        assert!(load_repo_key_with_passphrase(dir.path(), "wrong").is_err());
        let loaded = load_repo_key_with_passphrase(dir.path(), "strong-pass").unwrap();
        assert_eq!(key, loaded);
    }

    #[test]
    fn test_from_key_roundtrip() {
        let key = generate_repo_key();
        let cipher = RepoCipher::from_key(&key);
        let data = b"roundtrip via from_key";
        let encrypted = cipher.encrypt(data);
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);

        let cipher2 = RepoCipher::from_key(&key);
        let decrypted2 = cipher2.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted2, data);
    }
}
