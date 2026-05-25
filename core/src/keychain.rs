use anyhow::Result;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const RECORDS_DIR: &str = "records";
const ROTATIONS_DIR: &str = "rotations";
const ARCHIVE_DIR: &str = "archive";
const CURRENT_REF: &str = "current";

pub fn key_id_from_public_key(pk: &VerifyingKey) -> String {
    blake3::hash(&pk.to_bytes()).to_hex().to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyRecord {
    pub key_id: String,
    pub public_key_hex: String,
    pub created_at: u64,
    pub previous_key_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyRotation {
    pub rotation_id: String,
    pub old_key_id: String,
    pub new_key_id: String,
    pub new_public_key_hex: String,
    pub timestamp: u64,
    pub signature_hex: String,
}

impl KeyRotation {
    /// Verify that this rotation was signed by the old key's private key.
    pub fn verify(&self, old_public_key: &VerifyingKey) -> Result<()> {
        let payload = serde_json::json!({
            "old_key_id": self.old_key_id,
            "new_key_id": self.new_key_id,
            "new_public_key_hex": self.new_public_key_hex,
            "timestamp": self.timestamp,
        });
        let payload_bytes = serde_json::to_vec(&payload)?;
        let sig_bytes = hex::decode(&self.signature_hex)?;
        let signature = ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into()?);
        old_public_key.verify(&payload_bytes, &signature)?;
        Ok(())
    }
}

/// Initialize the keychain with the current key as the genesis root.
/// Must be called after the initial keypair is saved to `keys_dir`.
pub fn init_keychain(keys_dir: &Path) -> Result<String> {
    let pub_bytes = fs::read(keys_dir.join("public.key"))?;
    let pk = VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?;
    let key_id = key_id_from_public_key(&pk);

    fs::create_dir_all(keys_dir.join(RECORDS_DIR))?;
    fs::create_dir_all(keys_dir.join(ROTATIONS_DIR))?;
    fs::create_dir_all(keys_dir.join(ARCHIVE_DIR))?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let record = KeyRecord {
        key_id: key_id.clone(),
        public_key_hex: hex::encode(pk.to_bytes()),
        created_at: now,
        previous_key_id: None,
    };
    let record_path = keys_dir.join(RECORDS_DIR).join(format!("{}.json", key_id));
    fs::write(&record_path, serde_json::to_string_pretty(&record)?)?;

    set_current_key(keys_dir, &key_id)?;
    Ok(key_id)
}

/// Return the key_id of the currently active key.
pub fn get_current_key_id(keys_dir: &Path) -> Result<String> {
    let current_path = keys_dir.join(CURRENT_REF);
    let key_id = fs::read_to_string(&current_path)?.trim().to_string();
    if key_id.is_empty() {
        anyhow::bail!("current key ref is empty");
    }
    Ok(key_id)
}

fn set_current_key(keys_dir: &Path, key_id: &str) -> Result<()> {
    fs::write(keys_dir.join(CURRENT_REF), key_id)?;
    Ok(())
}

/// Generate a new ed25519 signing keypair, archive the old one, and
/// persist a signed rotation record.
pub fn rotate_signing_key(keys_dir: &Path) -> Result<KeyRotation> {
    let old_secret = fs::read(keys_dir.join("secret.key"))?;
    let old_signing_key = SigningKey::from_bytes(old_secret.as_slice().try_into()?);
    let old_verifying_key = old_signing_key.verifying_key();
    let old_key_id = get_current_key_id(keys_dir)?;

    // Generate new keypair
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    let mut csprng = rand::rngs::OsRng;
    csprng.fill_bytes(&mut bytes);
    let new_signing_key = SigningKey::from_bytes(&bytes);
    let new_verifying_key = new_signing_key.verifying_key();
    let new_key_id = key_id_from_public_key(&new_verifying_key);

    // Archive old key
    let archive_dir = keys_dir.join(ARCHIVE_DIR).join(&old_key_id);
    fs::create_dir_all(&archive_dir)?;
    fs::write(archive_dir.join("secret.key"), &old_secret)?;
    fs::write(archive_dir.join("public.key"), old_verifying_key.to_bytes())?;

    // Create and sign the rotation
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let new_pk_hex = hex::encode(new_verifying_key.to_bytes());
    let payload = serde_json::json!({
        "old_key_id": old_key_id,
        "new_key_id": new_key_id,
        "new_public_key_hex": new_pk_hex,
        "timestamp": now,
    });
    let payload_bytes = serde_json::to_vec(&payload)?;
    let signature = old_signing_key.sign(&payload_bytes);

    let rotation = KeyRotation {
        rotation_id: blake3::hash(&payload_bytes).to_hex().to_string(),
        old_key_id,
        new_key_id: new_key_id.clone(),
        new_public_key_hex: new_pk_hex,
        timestamp: now,
        signature_hex: hex::encode(signature.to_bytes()),
    };

    // Save rotation record
    let rotation_path = keys_dir
        .join(ROTATIONS_DIR)
        .join(format!("{}.json", rotation.rotation_id));
    fs::write(&rotation_path, serde_json::to_string_pretty(&rotation)?)?;

    // Create a record for the new key
    let new_record = KeyRecord {
        key_id: new_key_id.clone(),
        public_key_hex: hex::encode(new_verifying_key.to_bytes()),
        created_at: now,
        previous_key_id: Some(rotation.old_key_id.clone()),
    };
    let record_path = keys_dir
        .join(RECORDS_DIR)
        .join(format!("{}.json", new_key_id));
    fs::write(&record_path, serde_json::to_string_pretty(&new_record)?)?;

    // Save new keypair as current
    fs::write(keys_dir.join("secret.key"), new_signing_key.to_bytes())?;
    fs::write(keys_dir.join("public.key"), new_verifying_key.to_bytes())?;
    set_current_key(keys_dir, &new_key_id)?;

    Ok(rotation)
}

/// Walk the key rotation chain for a given key_id, returning all rotation
/// records in order (newest first). Starts from the rotation whose new_key_id
/// matches key_id, then follows previous_key_id backward to genesis.
pub fn collect_rotation_chain(keys_dir: &Path, key_id: &str) -> Result<Vec<KeyRotation>> {
    let rotations = load_rotations(keys_dir)?;
    // Build index: new_key_id -> rotation
    let new_to_old: std::collections::HashMap<&str, &KeyRotation> = rotations
        .iter()
        .map(|r| (r.new_key_id.as_str(), r))
        .collect();
    let mut chain = Vec::new();
    let mut current = key_id;
    while let Some(rot) = new_to_old.get(current) {
        chain.push((*rot).clone());
        current = &rot.old_key_id;
    }
    Ok(chain)
}

/// Load all rotation records sorted by timestamp.
pub fn load_rotations(keys_dir: &Path) -> Result<Vec<KeyRotation>> {
    let rot_dir = keys_dir.join(ROTATIONS_DIR);
    if !rot_dir.exists() {
        return Ok(Vec::new());
    }
    let mut rotations = Vec::new();
    for entry in fs::read_dir(&rot_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let data = fs::read(entry.path())?;
            if let Ok(rot) = serde_json::from_slice::<KeyRotation>(&data) {
                rotations.push(rot);
            }
        }
    }
    rotations.sort_by_key(|a| a.timestamp);
    Ok(rotations)
}

/// Load all key records sorted by creation time.
pub fn load_records(keys_dir: &Path) -> Result<Vec<KeyRecord>> {
    let rec_dir = keys_dir.join(RECORDS_DIR);
    if !rec_dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&rec_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let data = fs::read(entry.path())?;
            if let Ok(record) = serde_json::from_slice::<KeyRecord>(&data) {
                records.push(record);
            }
        }
    }
    records.sort_by_key(|a| a.created_at);
    Ok(records)
}

/// Walk every rotation and verify its signature. Returns a list of errors.
pub fn verify_keychain(keys_dir: &Path) -> Result<Vec<String>> {
    let rotations = load_rotations(keys_dir)?;
    let mut errors = Vec::new();
    for rotation in &rotations {
        let old_pk = resolve_public_key(keys_dir, &rotation.old_key_id)?;
        if let Err(e) = rotation.verify(&old_pk) {
            errors.push(format!("rotation {}: {}", rotation.rotation_id, e));
        }
    }
    Ok(errors)
}

/// Find the ed25519 public key for a given key_id by searching:
/// current key, archived keys, and key records.
pub fn resolve_public_key(keys_dir: &Path, key_id: &str) -> Result<VerifyingKey> {
    if let Ok(current_id) = get_current_key_id(keys_dir) {
        if current_id == key_id {
            let pub_bytes = fs::read(keys_dir.join("public.key"))?;
            return Ok(VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?);
        }
    }

    let archive_pub = keys_dir.join(ARCHIVE_DIR).join(key_id).join("public.key");
    if archive_pub.exists() {
        let pub_bytes = fs::read(&archive_pub)?;
        return Ok(VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?);
    }

    let records = load_records(keys_dir)?;
    for record in &records {
        if record.key_id == key_id {
            let pk_bytes = hex::decode(&record.public_key_hex)?;
            return Ok(VerifyingKey::from_bytes(pk_bytes.as_slice().try_into()?)?);
        }
    }

    anyhow::bail!("key_id {} not found in keychain", key_id)
}

/// Verify that `key_id` was an active (non-expired) key at the given
/// Unix timestamp (seconds).  Keychain timestamps are stored in
/// milliseconds, so we compare at second precision.
pub fn key_was_valid_at(keys_dir: &Path, key_id: &str, timestamp_secs: u64) -> Result<()> {
    let records = load_records(keys_dir)?;
    let record = records
        .iter()
        .find(|r| r.key_id == key_id)
        .ok_or_else(|| anyhow::anyhow!("key_id {} not found in keychain", key_id))?;

    // Compare at second precision (div 1000) to align with commit timestamps.
    let created_secs = record.created_at / 1000;
    if created_secs > timestamp_secs {
        anyhow::bail!(
            "key {} created at {} (secs) but commit is at {} — key not yet valid",
            key_id,
            created_secs,
            timestamp_secs
        );
    }

    for next in &records {
        if next.previous_key_id.as_deref() == Some(key_id) {
            let next_secs = next.created_at / 1000;
            // A key is valid for the entire second in which it rotates. Only
            // reject if the rotation finished *before* the commit second.
            if next_secs < timestamp_secs {
                anyhow::bail!(
                    "key {} rotated at {} (secs) but commit is at {} — key was already stale",
                    key_id,
                    next_secs,
                    timestamp_secs
                );
            }
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_initial_keypair(keys_dir: &Path) {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let sk = SigningKey::from_bytes(&bytes);
        let pk = sk.verifying_key();
        fs::write(keys_dir.join("secret.key"), sk.to_bytes()).unwrap();
        fs::write(keys_dir.join("public.key"), pk.to_bytes()).unwrap();
    }

    #[test]
    fn test_key_id_deterministic() {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let sk = SigningKey::from_bytes(&bytes);
        let pk = sk.verifying_key();
        assert_eq!(key_id_from_public_key(&pk), key_id_from_public_key(&pk));
    }

    #[test]
    fn test_init_keychain_creates_record_and_ref() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        let pub_bytes = fs::read(keys_dir.join("public.key")).unwrap();
        let pk = VerifyingKey::from_bytes(pub_bytes.as_slice().try_into().unwrap()).unwrap();

        let key_id = init_keychain(&keys_dir).unwrap();
        assert_eq!(key_id, key_id_from_public_key(&pk));

        let stored = fs::read_to_string(keys_dir.join("current")).unwrap();
        assert_eq!(stored.trim(), key_id);

        let rec_path = keys_dir.join(RECORDS_DIR).join(format!("{}.json", key_id));
        assert!(rec_path.exists());
    }

    #[test]
    fn test_rotate_signing_key_creates_rotation_and_updates_current() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        let old_key_id = init_keychain(&keys_dir).unwrap();

        let rotation = rotate_signing_key(&keys_dir).unwrap();
        assert_eq!(rotation.old_key_id, old_key_id);
        assert_ne!(rotation.new_key_id, old_key_id);

        let current_id = get_current_key_id(&keys_dir).unwrap();
        assert_eq!(current_id, rotation.new_key_id);

        let rot_path = keys_dir
            .join(ROTATIONS_DIR)
            .join(format!("{}.json", rotation.rotation_id));
        assert!(rot_path.exists());
    }

    #[test]
    fn test_rotation_verifies_with_old_key() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        init_keychain(&keys_dir).unwrap();
        let rotation = rotate_signing_key(&keys_dir).unwrap();

        let old_pk = resolve_public_key(&keys_dir, &rotation.old_key_id).unwrap();
        assert!(rotation.verify(&old_pk).is_ok());

        let errors = verify_keychain(&keys_dir).unwrap();
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn test_key_was_valid_at() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        let old_key_id = init_keychain(&keys_dir).unwrap();

        // Sleep to ensure rotation falls in a different second than init.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let rotation = rotate_signing_key(&keys_dir).unwrap();

        let rot_secs = rotation.timestamp / 1000;
        let old_created_secs = load_records(&keys_dir)
            .unwrap()
            .iter()
            .find(|r| r.key_id == old_key_id)
            .unwrap()
            .created_at
            / 1000;

        // Old key not valid before its creation second
        assert!(key_was_valid_at(&keys_dir, &old_key_id, old_created_secs - 1).is_err());

        // Old key valid at its creation second (which is < rot_secs with 1.5s sleep)
        assert!(key_was_valid_at(&keys_dir, &old_key_id, old_created_secs).is_ok());

        // Old key valid throughout the rotation second (grace window)
        assert!(key_was_valid_at(&keys_dir, &old_key_id, rot_secs).is_ok());

        // Old key invalid starting the second AFTER rotation
        assert!(key_was_valid_at(&keys_dir, &old_key_id, rot_secs + 1).is_err());

        // New key valid at rotation second onward
        assert!(key_was_valid_at(&keys_dir, &rotation.new_key_id, rot_secs).is_ok());

        // New key not valid before rotation second
        assert!(key_was_valid_at(&keys_dir, &rotation.new_key_id, rot_secs - 1).is_err());
    }

    #[test]
    fn test_resolve_public_key_after_rotation() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        let old_key_id = init_keychain(&keys_dir).unwrap();
        let rotation = rotate_signing_key(&keys_dir).unwrap();

        // Old key resolvable from archive
        let old_pk = resolve_public_key(&keys_dir, &old_key_id).unwrap();
        assert_eq!(key_id_from_public_key(&old_pk), old_key_id);

        // New key resolvable from current
        let new_pk = resolve_public_key(&keys_dir, &rotation.new_key_id).unwrap();
        assert_eq!(key_id_from_public_key(&new_pk), rotation.new_key_id);
    }

    #[test]
    fn test_tampered_rotation_is_detected() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        init_keychain(&keys_dir).unwrap();
        rotate_signing_key(&keys_dir).unwrap();

        // Tamper every rotation file
        let rot_dir = keys_dir.join(ROTATIONS_DIR);
        for entry in fs::read_dir(&rot_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                let data = fs::read(entry.path()).unwrap();
                if let Ok(mut rot) = serde_json::from_slice::<KeyRotation>(&data) {
                    rot.signature_hex = hex::encode([0u8; 64]);
                    fs::write(entry.path(), serde_json::to_string_pretty(&rot).unwrap()).unwrap();
                }
            }
        }

        let errors = verify_keychain(&keys_dir).unwrap();
        assert!(!errors.is_empty(), "tampered rotation must fail");
    }

    #[test]
    fn test_double_rotation() {
        let dir = tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        create_initial_keypair(&keys_dir);

        let key1 = init_keychain(&keys_dir).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(1500));
        let rot1 = rotate_signing_key(&keys_dir).unwrap();
        let key2 = rot1.new_key_id.clone();

        std::thread::sleep(std::time::Duration::from_millis(1500));
        let rot2 = rotate_signing_key(&keys_dir).unwrap();
        let key3 = rot2.new_key_id.clone();

        assert_ne!(key1, key2);
        assert_ne!(key2, key3);
        assert_ne!(key1, key3);

        let current = get_current_key_id(&keys_dir).unwrap();
        assert_eq!(current, key3);

        let errors = verify_keychain(&keys_dir).unwrap();
        assert!(errors.is_empty(), "{:?}", errors);

        let r1s = rot1.timestamp / 1000;
        let r2s = rot2.timestamp / 1000;

        // key1 valid throughout rot1 second (grace window)
        assert!(key_was_valid_at(&keys_dir, &key1, r1s).is_ok());
        // key1 invalid the second after rot1
        assert!(key_was_valid_at(&keys_dir, &key1, r1s + 1).is_err());
        // key2 valid during rot2 second
        assert!(key_was_valid_at(&keys_dir, &key2, r2s).is_ok());
        // key2 invalid the second after rot2
        assert!(key_was_valid_at(&keys_dir, &key2, r2s + 1).is_err());
        // key3 valid at rot2 second onward
        assert!(key_was_valid_at(&keys_dir, &key3, r2s).is_ok());
    }
}
