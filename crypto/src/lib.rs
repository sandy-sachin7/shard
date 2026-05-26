use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::Result;
use argon2::Argon2;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use std::fs;
use std::path::Path;

const SALT_SIZE: usize = 16;
const NONCE_SIZE: usize = 12;

/// Ed25519 keypair for signing and verifying Shard commits.
/// Generated on `shard init` and persisted to `.shard/keys/`.
#[derive(Debug)]
pub struct KeyPair {
    /// Ed25519 signing key — kept secret, used to sign commits.
    pub signing_key: SigningKey,
    /// Ed25519 verifying key — public, embedded in commits for cross-peer verification.
    pub verifying_key: VerifyingKey,
}

impl KeyPair {
    /// Generate a new random Ed25519 keypair using the OS random source.
    pub fn generate() -> Self {
        let mut csprng = OsRng;
        let mut bytes = [0u8; 32];
        use rand::RngCore;
        csprng.fill_bytes(&mut bytes);
        let signing_key = SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Save the keypair to `path/secret.key` (unencrypted) and `path/public.key`.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.save_with_passphrase(path, "")
    }

    /// Save the keypair with passphrase-based encryption.
    /// When passphrase is empty, stores unencrypted (backward compatible).
    pub fn save_with_passphrase(&self, path: &Path, passphrase: &str) -> Result<()> {
        let secret_bytes = if passphrase.is_empty() {
            self.signing_key.to_bytes().to_vec()
        } else {
            let salt = {
                let mut s = [0u8; SALT_SIZE];
                OsRng.fill_bytes(&mut s);
                s
            };
            let mut derived_key = [0u8; 32];
            Argon2::default()
                .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
                .map_err(|e| anyhow::anyhow!("key derivation failed: {}", e))?;

            let key = Key::<Aes256Gcm>::from_slice(&derived_key);
            let cipher = Aes256Gcm::new(key);
            let mut nonce_bytes = [0u8; NONCE_SIZE];
            OsRng.fill_bytes(&mut nonce_bytes);
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher
                .encrypt(nonce, self.signing_key.to_bytes().as_slice())
                .map_err(|e| anyhow::anyhow!("encryption failed: {:?}", e))?;

            let mut out = Vec::with_capacity(SALT_SIZE + NONCE_SIZE + ciphertext.len());
            out.extend_from_slice(&salt);
            out.extend_from_slice(&nonce_bytes);
            out.extend_from_slice(&ciphertext);
            out
        };
        fs::write(path.join("secret.key"), secret_bytes)?;
        let pub_bytes = self.verifying_key.to_bytes();
        fs::write(path.join("public.key"), pub_bytes)?;
        Ok(())
    }

    /// Load a keypair from `path/secret.key` and `path/public.key`.
    /// Attempts unencrypted load first; if the key is encrypted, requires a non-empty passphrase.
    pub fn load(path: &Path) -> Result<Self> {
        let secret_bytes = fs::read(path.join("secret.key"))?;
        // 32 bytes = unencrypted legacy format
        if secret_bytes.len() == 32 {
            return Self::load_from_bytes(&secret_bytes, path);
        }
        anyhow::bail!(
            "secret.key is encrypted but no passphrase provided. Use `shard unlock --passphrase <pass>` or provide --passphrase"
        );
    }

    /// Load a keypair with passphrase for decryption.
    pub fn load_with_passphrase(path: &Path, passphrase: &str) -> Result<Self> {
        let secret_bytes = fs::read(path.join("secret.key"))?;
        if secret_bytes.len() == 32 {
            return Self::load_from_bytes(&secret_bytes, path);
        }
        if passphrase.is_empty() {
            anyhow::bail!(
                "secret.key is encrypted but no passphrase provided. Use `shard unlock --passphrase <pass>`"
            );
        }
        let salt = &secret_bytes[..SALT_SIZE];
        let nonce = &secret_bytes[SALT_SIZE..SALT_SIZE + NONCE_SIZE];
        let ciphertext = &secret_bytes[SALT_SIZE + NONCE_SIZE..];

        let mut derived_key = [0u8; 32];
        Argon2::default()
            .hash_password_into(passphrase.as_bytes(), salt, &mut derived_key)
            .map_err(|e| anyhow::anyhow!("key derivation failed: {}", e))?;

        let key = Key::<Aes256Gcm>::from_slice(&derived_key);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("decryption failed: wrong passphrase or corrupted key"))?;

        let arr: [u8; 32] = plaintext.as_slice().try_into()?;
        let signing_key = SigningKey::from_bytes(&arr);
        let verifying_key = signing_key.verifying_key();
        // Verify against stored public key
        let pub_bytes = fs::read(path.join("public.key"))?;
        let expected_vk = VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?;
        if verifying_key.to_bytes() != expected_vk.to_bytes() {
            anyhow::bail!("decrypted key does not match stored public key");
        }
        Ok(Self {
            signing_key,
            verifying_key,
        })
    }

    fn load_from_bytes(secret_bytes: &[u8], path: &Path) -> Result<Self> {
        let pub_bytes = fs::read(path.join("public.key"))?;
        let signing_key = SigningKey::from_bytes(secret_bytes.try_into()?);
        let verifying_key = VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?;
        Ok(Self {
            signing_key,
            verifying_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_keypair_generate_roundtrip() {
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save(dir.path()).unwrap();
        let loaded = KeyPair::load(dir.path()).unwrap();
        assert_eq!(kp.verifying_key.to_bytes(), loaded.verifying_key.to_bytes());
        use ed25519_dalek::Signer;
        let sig = kp.signing_key.sign(b"test message");
        use ed25519_dalek::Verifier;
        assert!(loaded.verifying_key.verify(b"test message", &sig).is_ok());
    }

    #[test]
    fn test_keypair_generates_unique_keys() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        assert_ne!(kp1.verifying_key.to_bytes(), kp2.verifying_key.to_bytes());
    }

    #[test]
    fn test_keypair_sign_verify() {
        let kp = KeyPair::generate();
        use ed25519_dalek::{Signer, Verifier};
        let data = b"important data to sign";
        let sig = kp.signing_key.sign(data);
        assert!(kp.verifying_key.verify(data, &sig).is_ok());
    }

    #[test]
    fn test_keypair_wrong_key_rejects() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        use ed25519_dalek::{Signer, Verifier};
        let sig = kp1.signing_key.sign(b"data");
        assert!(kp2.verifying_key.verify(b"data", &sig).is_err());
    }

    #[test]
    fn test_keypair_load_nonexistent_fails() {
        let dir = tempfile::tempdir().unwrap();
        let result = KeyPair::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_keypair_save_with_passphrase_roundtrip() {
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save_with_passphrase(dir.path(), "hunter2").unwrap();
        let loaded = KeyPair::load_with_passphrase(dir.path(), "hunter2").unwrap();
        assert_eq!(kp.verifying_key.to_bytes(), loaded.verifying_key.to_bytes());
    }

    #[test]
    fn test_keypair_wrong_passphrase_fails() {
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save_with_passphrase(dir.path(), "correct").unwrap();
        let result = KeyPair::load_with_passphrase(dir.path(), "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn test_keypair_encrypted_load_without_passphrase_fails() {
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save_with_passphrase(dir.path(), "secret").unwrap();
        let result = KeyPair::load(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("encrypted"));
    }

    #[test]
    fn test_keypair_empty_passphrase_is_unencrypted() {
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save_with_passphrase(dir.path(), "").unwrap();
        let loaded = KeyPair::load(dir.path()).unwrap();
        assert_eq!(kp.verifying_key.to_bytes(), loaded.verifying_key.to_bytes());
    }

    #[test]
    fn test_keypair_encrypted_storage_format_size() {
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save_with_passphrase(dir.path(), "hunter2").unwrap();
        let secret = std::fs::read(dir.path().join("secret.key")).unwrap();
        // 16 (salt) + 12 (nonce) + 48 (ciphertext: 32 key + 16 tag) = 76
        assert_eq!(secret.len(), 76, "encrypted key should be 76 bytes");
    }
}
