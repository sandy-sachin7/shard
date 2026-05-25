use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::Result;
use rand::RngCore;
use std::fs;
use std::path::Path;

const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;

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

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .expect("encryption should never fail with given params");
        let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        result
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_SIZE {
            anyhow::bail!(
                "encrypted data too short (need {} bytes for nonce)",
                NONCE_SIZE
            );
        }
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("decryption failed — wrong key or corrupted data"))?;
        Ok(plaintext)
    }

    pub fn key_bytes(&self) -> [u8; KEY_SIZE] {
        // Aes256Gcm doesn't expose the key directly. We re-generate from a stored copy.
        // This is a placeholder — keys are managed externally via save/load functions.
        unimplemented!("use save_repo_key / load_repo_key instead")
    }
}

pub fn generate_repo_key() -> [u8; KEY_SIZE] {
    let mut key = [0u8; KEY_SIZE];
    OsRng.fill_bytes(&mut key);
    key
}

pub fn save_repo_key(keys_dir: &Path, key: &[u8; KEY_SIZE]) -> Result<()> {
    fs::write(keys_dir.join("repo.key"), hex::encode(key))?;
    Ok(())
}

pub fn load_repo_key(keys_dir: &Path) -> Result<[u8; KEY_SIZE]> {
    let hex_key = fs::read_to_string(keys_dir.join("repo.key"))?;
    let key_hex = hex_key.trim();
    let bytes = hex::decode(key_hex)?;
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
        // Flip a byte in the ciphertext portion (after nonce)
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
    fn test_from_key_roundtrip() {
        let key = generate_repo_key();
        let cipher = RepoCipher::from_key(&key);
        let data = b"roundtrip via from_key";
        let encrypted = cipher.encrypt(data);
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);

        // Same key should produce decryptable ciphertext
        let cipher2 = RepoCipher::from_key(&key);
        let decrypted2 = cipher2.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted2, data);
    }
}
