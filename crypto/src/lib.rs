//! Cryptographic primitives and key storage for the Shard distributed VCS.
//!
//! Provides Ed25519 [`KeyPair`] generation, atomic `0o600` secret persistence,
//! and Argon2id + AES-256-GCM passphrase encryption ([`encrypt_secret_bytes`] /
//! [`decrypt_secret_bytes`]) with automatic memory zeroization of key material.
//!
//! # Quickstart
//!
//! ```rust
//! use shard_crypto::KeyPair;
//!
//! # fn main() -> anyhow::Result<()> {
//! let dir = tempfile::tempdir()?;
//! let kp = KeyPair::generate();
//! kp.save_with_passphrase(dir.path(), "correct-horse-battery-staple")?;
//! let loaded = KeyPair::load_with_passphrase(dir.path(), "correct-horse-battery-staple")?;
//! assert_eq!(kp.verifying_key.to_bytes(), loaded.verifying_key.to_bytes());
//! # Ok(())
//! # }
//! ```

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::Result;
use argon2::Argon2;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use std::fs;
use std::io::Write;
use std::path::Path;
use zeroize::Zeroizing;

const SALT_SIZE: usize = 16;
const NONCE_SIZE: usize = 12;
const GCM_TAG_SIZE: usize = 16;
const MIN_ENCRYPTED_SIZE: usize = SALT_SIZE + NONCE_SIZE + GCM_TAG_SIZE;

/// Set restrictive `0o600` permissions on a secret file (Unix only).
pub fn set_secret_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Atomically write sensitive bytes to `path` with `0o600` permissions on Unix
/// set on file creation (before writing secret material) and `sync_all()` before persist.
/// Uses `tempfile::NamedTempFile::persist` so overwriting an existing target works
/// atomically on both Unix (`rename`) and Windows (`MoveFileExW`).
pub fn write_secret_file(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("secret path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("secret");

    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{}.tmp.", name))
        .tempfile_in(parent)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }

    tmp.as_file_mut().write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;

    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Atomically write non-secret bytes to `path` with `sync_all()` before persist.
/// Uses `tempfile::NamedTempFile::persist` so overwriting an existing target works
/// atomically on both Unix and Windows.
fn write_atomic_file(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("file path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{}.tmp.", name))
        .tempfile_in(parent)?;
    tmp.as_file_mut().write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;

    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Encrypt arbitrary secret bytes with Argon2id KDF + AES-256-GCM.
/// Output layout: `salt (16) || nonce (12) || ciphertext+tag`.
pub fn encrypt_secret_bytes(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if passphrase.is_empty() {
        anyhow::bail!("passphrase cannot be empty for encryption");
    }
    let mut salt = [0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    let mut derived_key = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), &salt, derived_key.as_mut())
        .map_err(|e| anyhow::anyhow!("key derivation failed: {}", e))?;

    let key = Key::<Aes256Gcm>::from_slice(derived_key.as_ref());
    let cipher = Aes256Gcm::new(key);
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("encryption failed: {:?}", e))?;

    let mut out = Vec::with_capacity(SALT_SIZE + NONCE_SIZE + ciphertext.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt secret bytes produced by [`encrypt_secret_bytes`].
/// Validates minimum ciphertext length before slicing and zeroizes intermediate key material.
pub fn decrypt_secret_bytes(data: &[u8], passphrase: &str) -> Result<Zeroizing<Vec<u8>>> {
    if data.len() < MIN_ENCRYPTED_SIZE {
        anyhow::bail!(
            "encrypted key data too short or corrupted (got {} bytes, need at least {})",
            data.len(),
            MIN_ENCRYPTED_SIZE
        );
    }
    if passphrase.is_empty() {
        anyhow::bail!("passphrase required to decrypt secret key");
    }
    let salt = &data[..SALT_SIZE];
    let nonce_bytes = &data[SALT_SIZE..SALT_SIZE + NONCE_SIZE];
    let ciphertext = &data[SALT_SIZE + NONCE_SIZE..];

    let mut derived_key = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, derived_key.as_mut())
        .map_err(|e| anyhow::anyhow!("key derivation failed: {}", e))?;

    let key = Key::<Aes256Gcm>::from_slice(derived_key.as_ref());
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("decryption failed: wrong passphrase or corrupted key"))?;

    Ok(Zeroizing::new(plaintext))
}

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
        let mut bytes = Zeroizing::new([0u8; 32]);
        csprng.fill_bytes(bytes.as_mut());
        let signing_key = SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Save the keypair to `path/secret.key` (unencrypted, `0o600` on Unix) and `path/public.key`.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.save_with_passphrase(path, "")
    }

    /// Save the keypair with passphrase-based encryption.
    /// When passphrase is empty, stores unencrypted (backward compatible) with `0o600` permissions.
    pub fn save_with_passphrase(&self, path: &Path, passphrase: &str) -> Result<()> {
        let raw_secret = Zeroizing::new(self.signing_key.to_bytes());
        let secret_bytes = if passphrase.is_empty() {
            Zeroizing::new(raw_secret.to_vec())
        } else {
            Zeroizing::new(encrypt_secret_bytes(raw_secret.as_ref(), passphrase)?)
        };
        write_secret_file(&path.join("secret.key"), &secret_bytes)?;
        let pub_bytes = self.verifying_key.to_bytes();
        write_atomic_file(&path.join("public.key"), &pub_bytes)?;
        Ok(())
    }

    /// Returns `true` if `path/secret.key` exists and is passphrase-encrypted.
    pub fn is_encrypted(path: &Path) -> Result<bool> {
        let secret_bytes = Zeroizing::new(fs::read(path.join("secret.key"))?);
        Ok(secret_bytes.len() != 32)
    }

    /// Load a keypair from `path/secret.key` and `path/public.key`.
    /// Attempts unencrypted load first; if the key is encrypted, requires a non-empty passphrase.
    pub fn load(path: &Path) -> Result<Self> {
        let secret_bytes = Zeroizing::new(fs::read(path.join("secret.key"))?);
        // 32 bytes = unencrypted format
        if secret_bytes.len() == 32 {
            return Self::load_from_bytes(&secret_bytes, path);
        }
        if secret_bytes.len() < MIN_ENCRYPTED_SIZE {
            anyhow::bail!(
                "secret.key is corrupted (expected 32 bytes unencrypted or >= {} bytes encrypted, got {})",
                MIN_ENCRYPTED_SIZE,
                secret_bytes.len()
            );
        }
        anyhow::bail!(
            "secret.key is encrypted but no passphrase provided. Set SHARD_PASSPHRASE or use --passphrase"
        );
    }

    /// Load a keypair with passphrase for decryption.
    pub fn load_with_passphrase(path: &Path, passphrase: &str) -> Result<Self> {
        let secret_bytes = Zeroizing::new(fs::read(path.join("secret.key"))?);
        if secret_bytes.len() == 32 {
            return Self::load_from_bytes(&secret_bytes, path);
        }
        if secret_bytes.len() < MIN_ENCRYPTED_SIZE {
            anyhow::bail!(
                "secret.key is corrupted (got {} bytes, need at least {})",
                secret_bytes.len(),
                MIN_ENCRYPTED_SIZE
            );
        }
        if passphrase.is_empty() {
            anyhow::bail!(
                "secret.key is encrypted but no passphrase provided. Set SHARD_PASSPHRASE or use --passphrase"
            );
        }

        let plaintext = decrypt_secret_bytes(&secret_bytes, passphrase)?;
        let arr: Zeroizing<[u8; 32]> = Zeroizing::new(
            plaintext
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("decrypted secret key has invalid length"))?,
        );
        let signing_key = SigningKey::from_bytes(&arr);
        let verifying_key = signing_key.verifying_key();

        // Verify against stored public key
        let pub_bytes = fs::read(path.join("public.key"))?;
        let expected_vk = VerifyingKey::from_bytes(
            pub_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("public.key has invalid length"))?,
        )?;
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
        let arr: Zeroizing<[u8; 32]> = Zeroizing::new(
            secret_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("secret.key has invalid length"))?,
        );
        let signing_key = SigningKey::from_bytes(&arr);
        let verifying_key = VerifyingKey::from_bytes(
            pub_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("public.key has invalid length"))?,
        )?;
        if signing_key.verifying_key().to_bytes() != verifying_key.to_bytes() {
            anyhow::bail!("secret.key does not match stored public.key");
        }
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
        use ed25519_dalek::{Signer, Verifier};
        let sig = kp.signing_key.sign(b"test message");
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

    #[test]
    fn test_keypair_short_corrupt_file_does_not_panic() {
        let dir = tempdir().unwrap();
        for len in [0usize, 1, 10, 27, 28, 43] {
            fs::write(dir.path().join("secret.key"), vec![0xAA; len]).unwrap();
            fs::write(dir.path().join("public.key"), [0u8; 32]).unwrap();
            assert!(KeyPair::load(dir.path()).is_err());
            assert!(KeyPair::load_with_passphrase(dir.path(), "pass").is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_secret_key_file_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;
        let kp = KeyPair::generate();
        let dir = tempdir().unwrap();
        kp.save(dir.path()).unwrap();
        let meta = fs::metadata(dir.path().join("secret.key")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn test_encrypt_decrypt_secret_bytes_roundtrip() {
        let secret = [0x42u8; 32];
        let enc = encrypt_secret_bytes(&secret, "my-pass").unwrap();
        let dec = decrypt_secret_bytes(&enc, "my-pass").unwrap();
        assert_eq!(dec.as_slice(), &secret);
        assert!(decrypt_secret_bytes(&enc, "wrong-pass").is_err());
        assert!(decrypt_secret_bytes(&[0u8; 10], "my-pass").is_err());
    }
}
