use crate::commit::Commit;
use crate::encryption;
use crate::error::ShardError;
use crate::fs_util::atomic_write;
use crate::keychain;
use crate::metadata;
use anyhow::Result;
use ed25519_dalek::{Verifier, VerifyingKey};
use shard_crypto::KeyPair;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::info;
use zeroize::Zeroizing;

static PASSPHRASE_CACHE: Mutex<Option<Zeroizing<String>>> = Mutex::new(None);

/// Cache a passphrase in-process (e.g., for `shard serve` or Python API sessions).
pub fn cache_passphrase(passphrase: &str) -> Result<()> {
    let mut cache = PASSPHRASE_CACHE
        .lock()
        .map_err(|_| anyhow::anyhow!("passphrase cache lock poisoned"))?;
    *cache = Some(Zeroizing::new(passphrase.to_string()));
    Ok(())
}

/// Retrieve the active passphrase from the in-process cache or `SHARD_PASSPHRASE` env var.
pub fn get_passphrase() -> Option<Zeroizing<String>> {
    if let Ok(cache) = PASSPHRASE_CACHE.lock() {
        if let Some(ref pass) = *cache {
            if !pass.is_empty() {
                return Some(pass.clone());
            }
        }
    }
    std::env::var("SHARD_PASSPHRASE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Zeroizing::new)
}

/// Validate that `passphrase` actually unlocks the repository's `secret.key` (and
/// `repo.key` if encrypted), then cache it in memory.
pub fn unlock_repo(path: &Path, passphrase: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        let keys_dir = shard_dir.join("keys");
        if keys_dir.join("secret.key").exists() {
            KeyPair::load_with_passphrase(&keys_dir, passphrase)?;
        }
        if keys_dir.join("repo.key").exists() {
            let _ = encryption::load_repo_key_with_passphrase(&keys_dir, passphrase)?;
        }
    }
    cache_passphrase(passphrase)
}

pub(crate) fn global_keys_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| Path::new(&h).join(".shard").join("keys"))
}

pub(crate) fn load_keypair(shard_dir: &Path) -> Result<KeyPair> {
    let local = shard_dir.join("keys");
    if local.join("secret.key").exists() {
        return load_keypair_from_dir(&local);
    }
    if let Some(global) = global_keys_dir() {
        if global.join("secret.key").exists() {
            return load_keypair_from_dir(&global);
        }
    }
    anyhow::bail!("no keypair found in {} or ~/.shard/keys/", local.display())
}

fn load_keypair_from_dir(dir: &Path) -> Result<KeyPair> {
    match KeyPair::load(dir) {
        Ok(kp) => Ok(kp),
        Err(e) => {
            if e.to_string().contains("encrypted") {
                if let Some(pass) = get_passphrase() {
                    return KeyPair::load_with_passphrase(dir, &pass);
                }
                anyhow::bail!(
                    "secret.key is encrypted. Set SHARD_PASSPHRASE or run `shard unlock`"
                );
            }
            Err(e)
        }
    }
}

pub(crate) fn load_public_key(shard_dir: &Path) -> Result<Vec<u8>> {
    let local = shard_dir.join("keys/public.key");
    if local.exists() {
        return Ok(fs::read(&local)?);
    }
    if let Some(global) = global_keys_dir() {
        let gp = global.join("public.key");
        if gp.exists() {
            return Ok(fs::read(&gp)?);
        }
    }
    anyhow::bail!(
        "no public key found in {} or ~/.shard/keys/",
        local.display()
    )
}

pub(crate) fn maybe_load_cipher(shard_dir: &Path) -> Result<Option<encryption::RepoCipher>> {
    let config = crate::load_config(shard_dir)?;
    if config.get("private").map(|s| s.as_str()) == Some("true") {
        let pass = get_passphrase();
        let pass_str = pass.as_deref().map(|s| s.as_str()).unwrap_or("");
        let key = Zeroizing::new(encryption::load_repo_key_with_passphrase(
            &shard_dir.join("keys"),
            pass_str,
        )?);
        Ok(Some(encryption::RepoCipher::from_key(&key)))
    } else {
        Ok(None)
    }
}

pub(crate) fn authorized_keys_path(shard_dir: &Path) -> PathBuf {
    shard_dir.join("authorized_keys")
}

/// An authorized key entry with optional comment.
pub(crate) struct AuthorizedKeyEntry {
    pub key: VerifyingKey,
    pub comment: String,
}

pub(crate) fn parse_authorized_key_line(line: &str) -> Option<AuthorizedKeyEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
    let hex_key = parts[0].trim();
    let comment = parts.get(1).unwrap_or(&"").trim().to_string();
    let bytes = hex::decode(hex_key).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    let key = VerifyingKey::from_bytes(&arr).ok()?;
    Some(AuthorizedKeyEntry { key, comment })
}

pub(crate) fn load_authorized_keys(shard_dir: &Path) -> Result<Vec<VerifyingKey>> {
    let path = authorized_keys_path(shard_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let mut keys = Vec::new();
    for line in content.lines() {
        if let Some(entry) = parse_authorized_key_line(line) {
            keys.push(entry.key);
        }
    }
    Ok(keys)
}

/// Collect the full set of locally trusted Ed25519 verifying keys for a repository:
/// local keychain (current + archived + records), `authorized_keys`, and global public key.
pub(crate) fn load_trusted_keys(shard_dir: &Path) -> Result<Vec<VerifyingKey>> {
    let mut trusted = keychain::load_all_local_public_keys(&shard_dir.join("keys"))?;
    for k in load_authorized_keys(shard_dir)? {
        if !trusted.contains(&k) {
            trusted.push(k);
        }
    }
    if let Some(global) = global_keys_dir() {
        let gp = global.join("public.key");
        if gp.exists() {
            if let Ok(bytes) = fs::read(&gp) {
                if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
                    if let Ok(vk) = VerifyingKey::from_bytes(&arr) {
                        if !trusted.contains(&vk) {
                            trusted.push(vk);
                        }
                    }
                }
            }
        }
    }
    Ok(trusted)
}

/// Cryptographically verify a commit's Ed25519 signature, `key_id` binding, keychain
/// validity window (when present locally), and trust anchor membership.
///
/// - When `allow_unsigned == false`, unsigned commits are rejected.
/// - When `require_trust_anchor == true`, the signing public key MUST exist in the local
///   keychain or `authorized_keys` (never blindly trusting self-asserted `commit.public_key`).
/// - Returns `Ok(Some(verifying_key))` when a valid signature was verified, or `Ok(None)`
///   only when `allow_unsigned == true` and the commit has no signature.
pub(crate) fn verify_commit_signature(
    shard_dir: &Path,
    commit_id: &str,
    commit: &Commit,
    allow_unsigned: bool,
    require_trust_anchor: bool,
) -> Result<Option<VerifyingKey>> {
    let sig_hex = match &commit.signature {
        Some(s) => s,
        None => {
            if allow_unsigned {
                return Ok(None);
            }
            return Err(ShardError::UnsignedCommit(commit_id.to_string()).into());
        }
    };

    let verifying_key = if let Some(pk_hex) = &commit.public_key {
        let pk_bytes = hex::decode(pk_hex)?;
        VerifyingKey::from_bytes(pk_bytes.as_slice().try_into().map_err(|_| {
            ShardError::SignatureVerificationFailed("invalid public key length".into())
        })?)?
    } else if let Some(kid) = &commit.key_id {
        keychain::resolve_public_key(&shard_dir.join("keys"), kid)?
    } else {
        let pub_bytes = load_public_key(shard_dir)?;
        VerifyingKey::from_bytes(pub_bytes.as_slice().try_into().map_err(|_| {
            ShardError::SignatureVerificationFailed("invalid local public key".into())
        })?)?
    };

    // Always enforce key_id == blake3(public_key) when key_id is present
    if let Some(kid) = &commit.key_id {
        let derived_kid = keychain::key_id_from_public_key(&verifying_key);
        if &derived_kid != kid {
            return Err(ShardError::SignatureVerificationFailed(format!(
                "commit key_id '{}' does not match public key digest '{}'",
                kid, derived_kid
            ))
            .into());
        }
        // Check keychain time-validity window when this key is tracked in the local keychain
        let keys_dir = shard_dir.join("keys");
        if keychain::resolve_public_key(&keys_dir, kid).is_ok() {
            if let Err(e) = keychain::key_was_valid_at(&keys_dir, kid, commit.timestamp) {
                anyhow::bail!("Keychain verification failed: {}", e);
            }
        }
    }

    // Anchor trust against local keychain / authorized_keys
    if require_trust_anchor {
        let trusted = load_trusted_keys(shard_dir)?;
        if !trusted.contains(&verifying_key) {
            return Err(ShardError::UntrustedKey(commit_id.to_string()).into());
        }
    } else if authorized_keys_path(shard_dir).exists() {
        // Even in pull/sync mode, if authorized_keys exists, enforce it strictly
        let trusted = load_trusted_keys(shard_dir)?;
        if !trusted.contains(&verifying_key) {
            return Err(ShardError::UntrustedKey(commit_id.to_string()).into());
        }
    }

    let mut unsigned_commit = commit.clone();
    unsigned_commit.signature = None;
    let json_unsigned = metadata::serialize_for_signing(&unsigned_commit)?;

    let sig_bytes = hex::decode(sig_hex)?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| ShardError::SignatureVerificationFailed("invalid signature length".into()))?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);

    verifying_key
        .verify(&json_unsigned, &signature)
        .map_err(|e| ShardError::SignatureVerificationFailed(e.to_string()))?;

    Ok(Some(verifying_key))
}

fn repo_root_from_shard_dir(shard_dir: &Path) -> Option<PathBuf> {
    if shard_dir.file_name() == Some(std::ffi::OsStr::new(".shard")) {
        let parent = shard_dir.parent()?;
        if parent.as_os_str().is_empty() {
            Some(PathBuf::from("."))
        } else {
            Some(parent.to_path_buf())
        }
    } else {
        None
    }
}

pub fn add_authorized_key(shard_dir: &Path, public_key_hex: &str) -> Result<()> {
    if let Some(repo_root) = repo_root_from_shard_dir(shard_dir) {
        return crate::with_write_op(&repo_root, "add_authorized_key", || {
            add_authorized_key_inner(shard_dir, public_key_hex)
        });
    }
    add_authorized_key_inner(shard_dir, public_key_hex)
}

fn add_authorized_key_inner(shard_dir: &Path, public_key_hex: &str) -> Result<()> {
    let bytes = hex::decode(public_key_hex.trim())?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Public key must be 32 bytes (64 hex chars)"))?;
    let _pk = VerifyingKey::from_bytes(&arr)?;

    let path = authorized_keys_path(shard_dir);
    let mut content = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if content.lines().any(|l| {
        parse_authorized_key_line(l)
            .map(|entry| entry.key.to_bytes() == arr)
            .unwrap_or(false)
    }) {
        info!("Key already authorized");
        return Ok(());
    }
    content.push_str(public_key_hex.trim());
    content.push('\n');
    atomic_write(&path, content.as_bytes())?;
    info!("Authorized key added");
    Ok(())
}

pub fn remove_authorized_key(shard_dir: &Path, public_key_hex: &str) -> Result<()> {
    if let Some(repo_root) = repo_root_from_shard_dir(shard_dir) {
        return crate::with_write_op(&repo_root, "remove_authorized_key", || {
            remove_authorized_key_inner(shard_dir, public_key_hex)
        });
    }
    remove_authorized_key_inner(shard_dir, public_key_hex)
}

fn remove_authorized_key_inner(shard_dir: &Path, public_key_hex: &str) -> Result<()> {
    let path = authorized_keys_path(shard_dir);
    if !path.exists() {
        anyhow::bail!("authorized_keys file does not exist");
    }
    let content = fs::read_to_string(&path)?;
    let normalized = public_key_hex.trim().to_lowercase();
    let new_content: Vec<&str> = content
        .lines()
        .filter(|l| {
            if let Some(entry) = parse_authorized_key_line(l) {
                hex::encode(entry.key.to_bytes()) != normalized
            } else {
                true
            }
        })
        .collect();
    if new_content.len() == content.lines().count() {
        anyhow::bail!("Key not found in authorized_keys");
    }
    let has_keys = new_content
        .iter()
        .any(|l| parse_authorized_key_line(l).is_some());
    if has_keys {
        atomic_write(&path, (new_content.join("\n") + "\n").as_bytes())?;
    } else {
        fs::remove_file(&path)?;
    }
    info!("Authorized key removed");
    Ok(())
}

pub fn list_authorized_keys(shard_dir: &Path, json: bool) -> Result<()> {
    if let Some(repo_root) = repo_root_from_shard_dir(shard_dir) {
        return crate::with_read_op(&repo_root, "list_authorized_keys", || {
            list_authorized_keys_inner(shard_dir, json)
        });
    }
    list_authorized_keys_inner(shard_dir, json)
}

fn list_authorized_keys_inner(shard_dir: &Path, json: bool) -> Result<()> {
    let path = authorized_keys_path(shard_dir);
    if !path.exists() {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"keys": []}))?
            );
        } else {
            println!("No authorized keys.");
        }
        return Ok(());
    }
    let content = fs::read_to_string(&path)?;
    let mut entries = Vec::new();
    for line in content.lines() {
        if let Some(entry) = parse_authorized_key_line(line) {
            entries.push(serde_json::json!({
                "key": hex::encode(entry.key.to_bytes()),
                "comment": entry.comment,
            }));
        }
    }
    if entries.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"keys": []}))?
            );
        } else {
            println!("No authorized keys.");
        }
        return Ok(());
    }
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"keys": entries}))?
        );
    } else {
        for e in &entries {
            let comment = e["comment"].as_str().unwrap_or("");
            if comment.is_empty() {
                println!("  {}", e["key"].as_str().unwrap_or(""));
            } else {
                println!("  {}  {}", e["key"].as_str().unwrap_or(""), comment);
            }
        }
    }
    Ok(())
}
