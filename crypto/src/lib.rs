use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use anyhow::Result;
use std::fs;
use std::path::Path;

pub struct KeyPair {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut csprng = OsRng;
        let mut bytes = [0u8; 32];
        use rand::RngCore;
        csprng.fill_bytes(&mut bytes);
        let signing_key = SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        Self { signing_key, verifying_key }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = self.signing_key.to_bytes();
        fs::write(path.join("secret.key"), bytes)?;
        let pub_bytes = self.verifying_key.to_bytes();
        fs::write(path.join("public.key"), pub_bytes)?;
        Ok(())
    }
}
