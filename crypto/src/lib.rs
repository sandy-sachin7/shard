use anyhow::Result;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use std::fs;
use std::path::Path;

/// Ed25519 keypair for signing and verifying Shard commits.
/// Generated on `shard init` and persisted to `.shard/keys/`.
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

    /// Save the keypair to `path/secret.key` and `path/public.key`.
    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = self.signing_key.to_bytes();
        fs::write(path.join("secret.key"), bytes)?;
        let pub_bytes = self.verifying_key.to_bytes();
        fs::write(path.join("public.key"), pub_bytes)?;
        Ok(())
    }

    /// Load a keypair from `path/secret.key` and `path/public.key`.
    pub fn load(path: &Path) -> Result<Self> {
        let secret_bytes = fs::read(path.join("secret.key"))?;
        let pub_bytes = fs::read(path.join("public.key"))?;

        let signing_key = SigningKey::from_bytes(secret_bytes.as_slice().try_into()?);
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
        // Verify signing consistency
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
}
