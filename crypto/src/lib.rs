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
