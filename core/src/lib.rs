pub mod branch;
pub mod chunker;
pub mod commit;
pub mod compression;
pub mod encryption;
pub mod index;
pub mod keychain;
pub mod manifest;
pub mod metadata;
pub mod partial;
pub mod store;
pub mod wal;

use crate::commit::Commit;
use crate::compression::Compression;
use crate::index::Index;
use crate::keychain::KeyRotation;
use crate::manifest::FileManifest;
use crate::store::Store;
use anyhow::Result;
use ed25519_dalek::{Signer, Verifier};
use metadata::MetadataFormat;
use serde::Serialize;
use shard_crypto::KeyPair;
use shard_net::libp2p::futures::StreamExt;
use similar::TextDiff;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info};

pub fn init(
    path: &Path,
    backend: &str,
    compression_algo: &str,
    chunker_mode: &str,
    chunk_size: Option<u64>,
    is_private: bool,
    json: bool,
) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!(
            "repository already initialized at {} (run `shard status` to confirm)",
            shard_dir.display()
        );
    }
    fs::create_dir_all(shard_dir.join("objects"))?;
    fs::create_dir_all(shard_dir.join("keys"))?;
    fs::create_dir_all(shard_dir.join("refs").join("heads"))?;
    branch::set_head_branch(&shard_dir, "main")?;

    let keys = KeyPair::generate();
    keys.save(&shard_dir.join("keys"))?;
    keychain::init_keychain(&shard_dir.join("keys"))?;

    // Generate a deterministic repo identity from the public key
    // (same key = same repo_id, so clones share the gossipsub topic)
    let pubkey = fs::read(shard_dir.join("keys/public.key"))?;
    let repo_id = blake3::hash(&pubkey).to_hex().to_string();
    let mut config = load_config(&shard_dir)?;

    if is_private {
        let key = encryption::generate_repo_key();
        encryption::save_repo_key(&shard_dir.join("keys"), &key)?;
        config.insert("private".to_string(), "true".to_string());
    }
    config.insert("repo_id".to_string(), repo_id);
    config.insert("storage_backend".to_string(), backend.to_string());
    config.insert(
        "serialization_format".to_string(),
        MetadataFormat::Json.config_value().to_string(),
    );
    config.insert("compression".to_string(), compression_algo.to_string());
    config.insert("chunker_mode".to_string(), chunker_mode.to_string());
    match chunker_mode {
        "rabin" => {
            let chunk_size = chunk_size.unwrap_or(4_194_304);
            let min = chunk_size / 4;
            let max = chunk_size * 2;
            config.insert("chunk_min".to_string(), min.to_string());
            config.insert("chunk_avg".to_string(), chunk_size.to_string());
            config.insert("chunk_max".to_string(), max.to_string());
        }
        _ => {
            let cs = chunk_size.unwrap_or(4_194_304);
            config.insert("chunk_size".to_string(), cs.to_string());
        }
    }
    save_config(&shard_dir, &config)?;

    let chunker_desc = if chunker_mode == "rabin" {
        format!(
            "rabin (avg {} bytes)",
            config.get("chunk_avg").unwrap_or(&"4 MiB".to_string())
        )
    } else {
        format!(
            "fixed ({} bytes)",
            config.get("chunk_size").unwrap_or(&"4 MiB".to_string())
        )
    };
    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "path": shard_dir.display().to_string(),
                "backend": backend,
                "compression": compression_algo,
                "chunker": chunker_desc,
                "private": is_private,
            }))?
        );
    } else {
        info!(
            "Initialized empty Shard repository in {} with {} storage (compression: {}, chunking: {})",
            shard_dir.display(),
            backend,
            compression_algo,
            chunker_desc,
        );
    }
    Ok(())
}

fn relative_path(repo_root: &Path, file_path: &Path) -> String {
    let repo = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let file = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    file.strip_prefix(&repo)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            file_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        })
}

fn detect_content_type(file_path: &Path) -> Option<String> {
    let ext = file_path.extension()?.to_str()?.to_lowercase();
    let mime = match ext.as_str() {
        "txt" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "yaml" | "yml" => "application/x-yaml",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "py" => "text/x-python",
        "rs" => "text/x-rust",
        "ts" => "text/x-typescript",
        "js" => "application/javascript",
        "wasm" => "application/wasm",
        "toml" => "application/toml",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "bin" => "application/octet-stream",
        "pt" | "pth" | "ckpt" | "safetensors" => "application/x-model",
        _ => return None,
    };
    Some(mime.to_string())
}

#[allow(clippy::too_many_arguments)]
fn add_file(
    repo_root: &Path,
    file_path: &Path,
    store: &Store,
    index: &mut Index,
    compression: &Compression,
    chunker_mode: &chunker::ChunkerMode,
    cipher: Option<&encryption::RepoCipher>,
    _json: bool,
) -> Result<()> {
    let file = fs::File::open(file_path)?;
    let mut chunker = match chunker_mode {
        chunker::ChunkerMode::Fixed { chunk_size } => {
            chunker::Chunker::new_fixed(Box::new(file), *chunk_size)
        }
        chunker::ChunkerMode::Rabin { min, avg, max } => {
            chunker::Chunker::new_rabin(Box::new(file), *min, *avg, *max)
        }
    };
    let mut chunk_hashes = Vec::new();
    let mut total_size = 0;

    while let Some(chunk) = chunker.next_chunk()? {
        let hash = chunk.hash;
        let compressed_data = compression.compress(&chunk.data)?;
        let stored_data = match cipher {
            Some(c) => c.encrypt(&compressed_data),
            None => compressed_data,
        };
        let stored = crate::chunker::Chunk {
            hash,
            data: stored_data,
            offset: chunk.offset,
        };
        store.put_chunk(&stored)?;
        chunk_hashes.push(hash.to_hex().to_string());
        total_size += chunk.data.len() as u64;
    }

    let name = relative_path(repo_root, file_path);
    let manifest = FileManifest {
        name: name.clone(),
        size: total_size,
        chunks: chunk_hashes.clone(),
        content_type: detect_content_type(file_path),
        compression: compression.as_str().to_string(),
        merkle_root: Some(FileManifest::merkle_root(&chunk_hashes)),
        created_by: None,
        created_at: None,
        signature: None,
    };

    index.files.insert(name.clone(), manifest);
    if !_json {
        info!("Added {} ({})", name, total_size);
    }
    Ok(())
}

pub fn recover(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    wal::recover(&shard_dir)?;
    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({"status": "recovery complete"}))?);
    } else {
        info!("Recovery complete.");
    }
    Ok(())
}

pub fn add(path: &Path, file_path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    wal::recover(&shard_dir)?;

    let config = load_config(&shard_dir)?;
    let compression: Compression = config
        .get("compression")
        .map(|s| s.as_str())
        .unwrap_or("zstd")
        .parse()?;

    let chunker_mode = chunker::ChunkerMode::from_config(&config);
    let fmt = MetadataFormat::from_config(&config);

    let store = Store::open(&shard_dir)?;
    let mut index = Index::load(&shard_dir.join("index"), &fmt)?;

    let cipher = maybe_load_cipher(&shard_dir)?;

    if file_path.is_dir() {
        for entry in walkdir::WalkDir::new(file_path)
            .into_iter()
            .filter_entry(|e| {
                e.file_name()
                    .to_str()
                    .map(|s| !s.starts_with('.'))
                    .unwrap_or(false)
            })
        {
            let entry = entry?;
            if entry.file_type().is_file() {
                add_file(
                    path,
                    entry.path(),
                    &store,
                    &mut index,
                    &compression,
                    &chunker_mode,
                    cipher.as_ref(),
                    json,
                )?;
            }
        }
    } else {
        add_file(
            path,
            file_path,
            &store,
            &mut index,
            &compression,
            &chunker_mode,
            cipher.as_ref(),
            json,
        )?;
    }

    index.save(&shard_dir.join("index"), &fmt)?;
    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({"status": "added"}))?);
    }
    Ok(())
}

pub fn commit(path: &Path, message: &str, author: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    // Recover from any previous crash before mutating
    wal::recover(&shard_dir)?;

    let config = load_config(&shard_dir)?;
    let fmt = MetadataFormat::from_config(&config);

    let store = Store::open(&shard_dir)?;
    let mut index = Index::load(&shard_dir.join("index"), &fmt)?;

    if index.files.is_empty() {
        anyhow::bail!("nothing to commit (stage files with `shard add` first)");
    }

    let head_path = shard_dir.join("HEAD");

    // WAL: back up pre-commit state
    let wal = wal::Wal::new(&shard_dir);
    let head_backup = fs::read_to_string(&head_path).ok();
    let index_backup = fs::read(shard_dir.join("index"))?;
    wal.append(&wal::WalEntry::CommitBegin {
        head_backup,
        index_backup,
    })?;

    // 1. Store manifests (signed)
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let keys = KeyPair::load(&shard_dir.join("keys"))?;
    let signing_key = keys.signing_key;
    let mut manifest_ids = Vec::new();
    for manifest in index.files.values_mut() {
        manifest.created_by = Some(author.to_string());
        manifest.created_at = Some(timestamp);

        let mut unsigned = manifest.clone();
        unsigned.signature = None;
        let canonical = metadata::serialize_for_signing(&unsigned);
        let sig = signing_key.sign(&canonical);
        manifest.signature = Some(hex::encode(sig.to_bytes()));

        let encoded = metadata::serialize(manifest, &fmt);
        let hash = blake3::hash(&encoded);
        let chunk = crate::chunker::Chunk {
            hash,
            data: encoded,
            offset: 0,
        };
        store.put_chunk(&chunk)?;
        manifest_ids.push(hash.to_hex().to_string());
    }
    manifest_ids.sort();

    // 2. Get parent
    let mut parents = Vec::new();
    let (current_branch, parent_id) = branch::resolve_head(&shard_dir)?;
    if let Some(pid) = parent_id {
        parents.push(pid);
    }

    // 3. Create commit
    let public_key_hex = hex::encode(keys.verifying_key.to_bytes());
    let key_id = keychain::get_current_key_id(&shard_dir.join("keys")).ok();
    let mut commit = Commit {
        commit_id: String::new(),
        parents,
        manifests: manifest_ids,
        author: author.to_string(),
        message: message.to_string(),
        timestamp,
        public_key: Some(public_key_hex),
        signature: None,
        key_id,
    };

    // 4. Sign — always JSON for deterministic signature
    let json_unsigned = metadata::serialize_for_signing(&commit);
    let signature = signing_key.sign(&json_unsigned);
    commit.signature = Some(hex::encode(signature.to_bytes()));

    // 5. Store commit — use configured format
    let encoded = metadata::serialize(&commit, &fmt);
    let hash = blake3::hash(&encoded);
    let chunk = crate::chunker::Chunk {
        hash,
        data: encoded,
        offset: 0,
    };
    store.put_chunk(&chunk)?;

    // 6. Cycle detection: verify no parent chain already contains this commit
    let commit_id = hash.to_hex().to_string();
    if has_dag_cycle(&store, &commit.parents, &commit_id)? {
        anyhow::bail!(
            "Cycle detected: commit {} is already an ancestor of one or more parents",
            commit_id
        );
    }

    // 7. Update HEAD and branch ref
    if let Some(ref branch_name) = current_branch {
        branch::update_branch_ref(&shard_dir, branch_name, &commit_id)?;
        branch::set_head_branch(&shard_dir, branch_name)?;
    } else {
        fs::write(&head_path, &commit_id)?;
    }

    // 7. Clear index
    index.files.clear();
    index.save(&shard_dir.join("index"), &fmt)?;

    // WAL: mark commit complete
    wal.append(&wal::WalEntry::CommitEnd)?;
    wal.truncate()?;

    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "commit_id": commit_id,
            "message": message,
        }))?);
    } else {
        info!("Committed {} ({})", commit_id, message);
    }
    Ok(())
}

pub fn verify(path: &Path, commit_id: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    if commit_id.len() < 2 {
        anyhow::bail!("invalid commit id (too short: need at least 2 characters)");
    }
    let store = Store::open(&shard_dir)?;
    let cipher = maybe_load_cipher(&shard_dir)?;
    let commit_data = store.get_chunk(commit_id)?;

    // Self-verify: stored blob hash must equal commit_id (M5)
    let stored_hash = blake3::hash(&commit_data);
    if stored_hash.to_hex().to_string() != commit_id {
        anyhow::bail!("commit object hash mismatch: stored content does not match its hash — data may be corrupted");
    }

    let commit: Commit = metadata::deserialize(&commit_data)?;

    let mut sig_verified = false;
    let mut files_checked = 0u64;

    // Verify the signing key was valid at commit time
    if let Some(kid) = &commit.key_id {
        if let Err(e) = keychain::key_was_valid_at(&shard_dir.join("keys"), kid, commit.timestamp) {
            anyhow::bail!("Keychain verification failed: {}", e);
        } else if !json {
            info!("Keychain: key {} was active at commit time.", kid);
        }
    }

    if let Some(sig_hex) = &commit.signature {
        let verifying_key = if let Some(pk_hex) = &commit.public_key {
            let pk_bytes = hex::decode(pk_hex)?;
            ed25519_dalek::VerifyingKey::from_bytes(pk_bytes.as_slice().try_into()?)?
        } else {
            let pub_key_path = shard_dir.join("keys/public.key");
            let pub_bytes = fs::read(pub_key_path)?;
            ed25519_dalek::VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?
        };

        let mut unsigned_commit = commit.clone();
        unsigned_commit.signature = None;
        let json_unsigned = metadata::serialize_for_signing(&unsigned_commit);

        let sig_bytes = hex::decode(sig_hex)?;
        let signature = ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into()?);

        verifying_key.verify(&json_unsigned, &signature)?;
        sig_verified = true;
        if !json {
            info!("Signature verified.");
        }
    } else if !json {
        info!("Warning: Commit is unsigned.");
    }

    for manifest_id in &commit.manifests {
        let manifest_data = store.get_chunk(manifest_id)?;
        let hash = blake3::hash(&manifest_data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("manifest object hash mismatch for manifest '{}': content does not match stored hash. The object store may be corrupted.", manifest_id);
        }

        let manifest: FileManifest = metadata::deserialize(&manifest_data)?;

        // Verify manifest signature (defense-in-depth; commit signature already covers manifest_id)
        if let Some(sig_hex) = &manifest.signature {
            let pk_bytes = if let Some(pk_hex) = &commit.public_key {
                hex::decode(pk_hex)?
            } else {
                let pub_key_path = shard_dir.join("keys/public.key");
                fs::read(pub_key_path)?
            };
            let vk = ed25519_dalek::VerifyingKey::from_bytes(pk_bytes.as_slice().try_into()?)?;
            let mut unsigned = manifest.clone();
            unsigned.signature = None;
            let canonical = metadata::serialize_for_signing(&unsigned);
            let sig_bytes = hex::decode(sig_hex)?;
            let sig = ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into()?);
            vk.verify(&canonical, &sig)?;
            if !json {
                info!("  Manifest signature verified for: {}", manifest.name);
            }
        }

        let compression = manifest.compression.parse::<Compression>()?;
        if !json {
            info!(
                "Verifying file: {} (compression: {})",
                manifest.name, manifest.compression
            );
        }

        // Verify merkle_root if present
        if let Some(ref mr) = manifest.merkle_root {
            let computed = FileManifest::merkle_root(&manifest.chunks);
            if mr != &computed {
                anyhow::bail!(
                    "merkle root mismatch for '{}': manifest says {} but computed {}",
                    manifest.name,
                    mr,
                    computed
                );
            }
        }

        for chunk_id in &manifest.chunks {
            let chunk_data = store.get_chunk(chunk_id)?;
            let decrypted = match &cipher {
                Some(c) => c.decrypt(&chunk_data)?,
                None => chunk_data,
            };
            let decompressed = compression.decompress(&decrypted)?;
            let hash = blake3::hash(&decompressed);
            if hash.to_hex().to_string() != *chunk_id {
                anyhow::bail!("chunk hash mismatch for '{}': content does not match stored hash (expected {}, got {}). File may be corrupted.", manifest.name, chunk_id, hash.to_hex());
            }
        }
        files_checked += 1;
    }

    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit_id": commit_id,
                "verified": true,
                "signature_verified": sig_verified,
                "files_checked": files_checked,
            }))?
        );
    } else {
        info!("Verification successful.");
    }
    Ok(())
}

fn load_commit(store: &Store, commit_id: &str) -> Result<Commit> {
    if commit_id.len() < 2 {
        anyhow::bail!(
            "commit id too short (got {} chars, need at least 2): '{}'",
            commit_id.len(),
            commit_id
        );
    };
    let data = store.get_chunk(commit_id)?;
    let mut commit: Commit = metadata::deserialize(&data)?;
    commit.commit_id = commit_id.to_string();
    Ok(commit)
}

fn has_dag_cycle(store: &Store, parents: &[String], commit_id: &str) -> Result<bool> {
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<String> = parents.to_vec();
    while let Some(cid) = stack.pop() {
        if cid == commit_id {
            return Ok(true);
        }
        if !seen.insert(cid.clone()) {
            continue;
        }
        if let Ok(data) = store.get_chunk(&cid) {
            if let Ok(commit) = metadata::deserialize::<Commit>(&data) {
                for p in &commit.parents {
                    stack.push(p.clone());
                }
            }
        }
    }
    Ok(false)
}

#[derive(Serialize)]
pub struct LogEntry {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub manifests: Vec<String>,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub signature: Option<String>,
}

impl From<Commit> for LogEntry {
    fn from(c: Commit) -> Self {
        LogEntry {
            commit_id: c.commit_id,
            parents: c.parents,
            manifests: c.manifests,
            author: c.author,
            message: c.message,
            timestamp: c.timestamp,
            signature: c.signature,
        }
    }
}

pub fn log_cmd(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let store = Store::open(&shard_dir)?;

    let (_, head_commit) = branch::resolve_head(&shard_dir)?;
    let head = head_commit
        .ok_or_else(|| anyhow::anyhow!("no commits yet (run `shard commit` after adding files)"))?;

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![head];

    while let Some(cid) = stack.pop() {
        if !seen.insert(cid.clone()) {
            continue;
        }
        let commit = load_commit(&store, &cid)?;
        for parent in &commit.parents {
            stack.push(parent.clone());
        }
        entries.push(commit.into());
    }

    if json {
        info!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for entry in &entries {
            let ts = {
                let secs = entry.timestamp as i64;
                let tm = time::OffsetDateTime::from_unix_timestamp(secs)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
                tm.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| entry.timestamp.to_string())
            };
            info!("commit {}", entry.commit_id);
            if !entry.parents.is_empty() {
                info!("parents: {}", entry.parents.join(" "));
            }
            info!("author: {}", entry.author);
            info!("date:   {}", ts);
            info!("");
            for line in entry.message.lines() {
                info!("    {}", line);
            }
            info!("");
        }
    }

    Ok(())
}

fn reconstruct_file(
    store: &Store,
    manifest: &FileManifest,
    cipher: Option<&encryption::RepoCipher>,
) -> Result<Vec<u8>> {
    let compression: Compression = manifest.compression.parse()?;
    let mut data = Vec::new();
    for chunk_id in &manifest.chunks {
        let chunk_data = store.get_chunk(chunk_id)?;
        let decrypted = match cipher {
            Some(c) => c.decrypt(&chunk_data)?,
            None => chunk_data,
        };
        let decompressed = compression.decompress(&decrypted)?;
        data.extend_from_slice(&decompressed);
    }
    Ok(data)
}

pub fn diff(path: &Path, commit_a: &str, commit_b: Option<&str>, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let store = Store::open(&shard_dir)?;
    let cipher = maybe_load_cipher(&shard_dir)?;

    let cid_b = match commit_b {
        Some(c) => branch::resolve_rev(&shard_dir, c)?,
        None => {
            let (_, head) = branch::resolve_head(&shard_dir)?;
            head.ok_or_else(|| anyhow::anyhow!("no commits yet"))?
        }
    };
    let cid_a = branch::resolve_rev(&shard_dir, commit_a)?;

    let commit1 = load_commit(&store, &cid_a)?;
    let commit2 = load_commit(&store, &cid_b)?;

    let mut files1: HashMap<String, FileManifest> = HashMap::new();
    for mid in &commit1.manifests {
        let data = store.get_chunk(mid)?;
        let m: FileManifest = metadata::deserialize(&data)?;
        files1.insert(m.name.clone(), m);
    }

    let mut files2: HashMap<String, FileManifest> = HashMap::new();
    for mid in &commit2.manifests {
        let data = store.get_chunk(mid)?;
        let m: FileManifest = metadata::deserialize(&data)?;
        files2.insert(m.name.clone(), m);
    }

    let mut all_names: Vec<&String> = files1.keys().chain(files2.keys()).collect();
    all_names.sort();
    all_names.dedup();

    let mut changes: Vec<serde_json::Value> = Vec::new();
    let mut diff_found = false;

    for name in all_names {
        match (files1.get(name), files2.get(name)) {
            (None, Some(manifest)) => {
                let content = reconstruct_file(&store, manifest, cipher.as_ref())?;
                let text = String::from_utf8_lossy(&content);
                diff_found = true;
                if json {
                    changes.push(serde_json::json!({
                        "type": "added",
                        "file": name,
                        "lines": text.lines().collect::<Vec<_>>(),
                    }));
                } else {
                    info!("--- /dev/null");
                    info!("+++ b/{}", name);
                    let lines: Vec<&str> = text.lines().collect();
                    info!("@@ -0,0 +1,{} @@", lines.len());
                    for line in &lines {
                        info!("+{}", line);
                    }
                }
            }
            (Some(manifest), None) => {
                let content = reconstruct_file(&store, manifest, cipher.as_ref())?;
                let text = String::from_utf8_lossy(&content);
                diff_found = true;
                if json {
                    changes.push(serde_json::json!({
                        "type": "removed",
                        "file": name,
                        "lines": text.lines().collect::<Vec<_>>(),
                    }));
                } else {
                    info!("--- a/{}", name);
                    info!("+++ /dev/null");
                    let lines: Vec<&str> = text.lines().collect();
                    info!("@@ -1,{} +0,0 @@", lines.len());
                    for line in &lines {
                        info!("-{}", line);
                    }
                }
            }
            (Some(ma), Some(mb)) => {
                if ma.chunks == mb.chunks {
                    continue;
                }
                let content_a = reconstruct_file(&store, ma, cipher.as_ref())?;
                let content_b = reconstruct_file(&store, mb, cipher.as_ref())?;
                diff_found = true;
                if json {
                    let text_a = String::from_utf8_lossy(&content_a);
                    let text_b = String::from_utf8_lossy(&content_b);
                    changes.push(serde_json::json!({
                        "type": "modified",
                        "file": name,
                        "old_lines": text_a.lines().collect::<Vec<_>>(),
                        "new_lines": text_b.lines().collect::<Vec<_>>(),
                    }));
                } else {
                    let text_a = String::from_utf8_lossy(&content_a);
                    let text_b = String::from_utf8_lossy(&content_b);
                    let diff = TextDiff::from_lines(text_a.as_ref(), text_b.as_ref());
                    let mut buf: Vec<u8> = Vec::new();
                    diff.unified_diff()
                        .header(&format!("a/{}", name), &format!("b/{}", name))
                        .to_writer(&mut buf)
                        .map_err(|e| anyhow::anyhow!("diff output error: {}", e))?;
                    let output = String::from_utf8_lossy(&buf);
                    for line in output.lines() {
                        info!("{}", line);
                    }
                }
            }
            (None, None) => {}
        }
    }

    if !diff_found {
        if json {
            info!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"changes": changes, "message": "no differences"})
                )?
            );
        } else {
            info!("No differences between the commits.");
        }
        return Ok(());
    }

    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({"changes": changes}))?
        );
    }

    Ok(())
}

pub fn checkout(path: &Path, target: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let store = Store::open(&shard_dir)?;
    let cipher = maybe_load_cipher(&shard_dir)?;

    // Resolve target: branch name or commit id
    let branch_path = shard_dir.join("refs").join("heads").join(target);
    let commit_id = if branch_path.exists() {
        let id = fs::read_to_string(&branch_path)?.trim().to_string();
        branch::set_head_branch(&shard_dir, target)?;
        id
    } else {
        branch::set_head_commit(&shard_dir, target)?;
        target.to_string()
    };

    let commit = load_commit(&store, &commit_id)?;
    let mut files = Vec::new();

    for manifest_id in &commit.manifests {
        let data = store.get_chunk(manifest_id)?;
        let hash = blake3::hash(&data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }
        let manifest: FileManifest = metadata::deserialize(&data)?;
        let compression = manifest.compression.parse::<Compression>()?;
        if !json {
            info!(
                "Checking out file: {} (compression: {})",
                manifest.name, manifest.compression
            );
        }

        let mut file_data = Vec::new();
        for chunk_id in &manifest.chunks {
            let chunk_data = store.get_chunk(chunk_id)?;
            let decrypted = match &cipher {
                Some(c) => c.decrypt(&chunk_data)?,
                None => chunk_data,
            };
            let decompressed = compression.decompress(&decrypted)?;
            file_data.extend_from_slice(&decompressed);
        }
        fs::write(path.join(&manifest.name), file_data)?;
        if !json {
            info!("  -> {}", manifest.name);
        }
        files.push(manifest.name);
    }

    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit_id": commit_id,
                "files": files,
            }))?
        );
    } else {
        info!("Checkout complete.");
    }
    Ok(())
}

pub fn status(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let config = load_config(&shard_dir)?;
    let fmt = MetadataFormat::from_config(&config);

    let (current_branch, head_commit) = branch::resolve_head(&shard_dir)?;
    let mut commit_id: Option<String> = None;
    if let Some(cid) = head_commit {
        commit_id = Some(cid);
        if !json {
            if let Some(ref branch) = current_branch {
                info!("On branch: {}", branch);
            } else {
                info!("HEAD detached at {}", commit_id.as_ref().unwrap());
            }
        }
    } else if !json {
        info!("No commits yet.");
    }

    let index = Index::load(&shard_dir.join("index"), &fmt)?;
    let staged: Vec<String> = index.files.keys().cloned().collect();
    if !json {
        if staged.is_empty() {
            info!("Nothing staged.");
        } else {
            info!("\nStaged files:");
            for name in &staged {
                info!("  {} (to be committed)", name);
            }
        }
    }

    let store = Store::open(&shard_dir)?;
    let mut deleted = Vec::new();
    let tracked_names: std::collections::HashSet<String> = if let Some(head) = &commit_id {
        let mut names = std::collections::HashSet::new();
        if let Ok(commit) = load_commit(&store, head) {
            for manifest_id in &commit.manifests {
                if let Ok(data) = store.get_chunk(manifest_id) {
                    if let Ok(manifest) = metadata::deserialize::<FileManifest>(&data) {
                        let file_path = path.join(&manifest.name);
                        if !file_path.exists() {
                            deleted.push(manifest.name.clone());
                        }
                        names.insert(manifest.name);
                    }
                }
            }
        }
        names
    } else {
        std::collections::HashSet::new()
    };

    if !json && !deleted.is_empty() {
        info!("\nDeleted files:");
        for name in &deleted {
            info!("  {} (deleted)", name);
        }
    }

    let mut untracked = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(ftype) = entry.file_type() {
                if ftype.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with('.')
                        && !index.files.contains_key(&name)
                        && !tracked_names.contains(&name)
                    {
                        untracked.push(name);
                    }
                }
            }
        }
    }
    if !json && !untracked.is_empty() {
        info!("\nUntracked files:");
        for name in &untracked {
            info!("  {}", name);
        }
    }

    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit": commit_id,
                "staged": staged,
                "deleted": deleted,
                "untracked": untracked,
            }))?
        );
    }

    Ok(())
}

fn load_config(shard_dir: &Path) -> Result<std::collections::BTreeMap<String, String>> {
    let config_path = shard_dir.join("config.json");
    if config_path.exists() {
        let data = fs::read(&config_path)?;
        Ok(metadata::deserialize(&data)?)
    } else {
        Ok(std::collections::BTreeMap::new())
    }
}

fn save_config(
    shard_dir: &Path,
    config: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let fmt = MetadataFormat::from_config(config);
    let data = metadata::serialize(config, &fmt);
    fs::write(shard_dir.join("config.json"), data)?;
    Ok(())
}

fn maybe_load_cipher(shard_dir: &Path) -> Result<Option<encryption::RepoCipher>> {
    let config = load_config(shard_dir)?;
    if config.get("private").map(|s| s.as_str()) == Some("true") {
        let key = encryption::load_repo_key(&shard_dir.join("keys"))?;
        Ok(Some(encryption::RepoCipher::from_key(&key)))
    } else {
        Ok(None)
    }
}

pub fn config_get(path: &Path, key: Option<&str>) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let config = load_config(&shard_dir)?;
    if let Some(key) = key {
        match config.get(key) {
            Some(value) => info!("{} = {}", key, value),
            None => anyhow::bail!("config key not found: {}", key),
        }
    } else {
        for (k, v) in &config {
            info!("{} = {}", k, v);
        }
    }
    Ok(())
}

pub fn config_set(path: &Path, key: &str, value: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let mut config = load_config(&shard_dir)?;
    config.insert(key.to_string(), value.to_string());
    save_config(&shard_dir, &config)?;
    info!("{} = {}", key, value);
    Ok(())
}

fn load_tags(shard_dir: &Path) -> Result<std::collections::BTreeMap<String, String>> {
    let tags_path = shard_dir.join("tags.json");
    if tags_path.exists() {
        let data = fs::read(&tags_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(std::collections::BTreeMap::new())
    }
}

fn save_tags(shard_dir: &Path, tags: &std::collections::BTreeMap<String, String>) -> Result<()> {
    let data = serde_json::to_string_pretty(tags)?;
    fs::write(shard_dir.join("tags.json"), data)?;
    Ok(())
}

pub fn tag_add(path: &Path, name: &str, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    // Verify commit exists
    let store = Store::open(&shard_dir)?;
    load_commit(&store, commit_id)?;
    let mut tags = load_tags(&shard_dir)?;
    tags.insert(name.to_string(), commit_id.to_string());
    save_tags(&shard_dir, &tags)?;
    info!("Tagged '{}' -> {}", name, commit_id);
    Ok(())
}

pub fn branch_create(path: &Path, name: &str, commit_id: Option<&str>) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let id = match commit_id {
        Some(cid) => cid.to_string(),
        None => {
            let (_, head) = branch::resolve_head(&shard_dir)?;
            head.ok_or_else(|| anyhow::anyhow!("No commits yet — cannot create branch"))?
        }
    };
    // Verify commit exists
    let store = Store::open(&shard_dir)?;
    load_commit(&store, &id)?;
    branch::create_branch(&shard_dir, name, &id)
}

pub fn branch_delete(path: &Path, name: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    branch::delete_branch(&shard_dir, name)
}

pub fn branch_list(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let (current, branches) = branch::list_branches(&shard_dir)?;
    if branches.is_empty() {
        info!("No branches.");
        return Ok(());
    }
    for (name, commit_id) in &branches {
        let prefix = if current.as_deref() == Some(name) {
            "* "
        } else {
            "  "
        };
        info!(
            "{}{} ({})",
            prefix,
            name,
            &commit_id[..8.min(commit_id.len())]
        );
    }
    Ok(())
}

pub fn merge(path: &Path, branch: &str, message: &str, author: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let store = Store::open(&shard_dir)?;

    let config = load_config(&shard_dir)?;
    let fmt = MetadataFormat::from_config(&config);

    // Resolve current HEAD
    let (current_branch, current_id) = branch::resolve_head(&shard_dir)?;
    let current_id =
        current_id.ok_or_else(|| anyhow::anyhow!("No commits yet — nothing to merge into"))?;

    // Resolve source branch
    let source_id = branch::resolve_rev(&shard_dir, branch)?;
    if source_id == current_id {
        anyhow::bail!("Already up to date — source is the same commit as HEAD");
    }

    // Load both commits
    let current_commit = load_commit(&store, &current_id)?;
    let source_commit = load_commit(&store, &source_id)?;

    // Load manifests from both sides
    let mut merged_manifests: std::collections::HashMap<String, (String, Vec<String>)> =
        std::collections::HashMap::new();

    for manifest_id in &current_commit.manifests {
        let data = store.get_chunk(manifest_id)?;
        let manifest: FileManifest = metadata::deserialize(&data)?;
        merged_manifests.insert(manifest.name.clone(), (manifest.name, manifest.chunks));
    }

    for manifest_id in &source_commit.manifests {
        let data = store.get_chunk(manifest_id)?;
        let manifest: FileManifest = metadata::deserialize(&data)?;
        merged_manifests.insert(manifest.name.clone(), (manifest.name, manifest.chunks));
    }

    // Store merged manifests (signed)
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let keys = KeyPair::load(&shard_dir.join("keys"))?;
    let signing_key = keys.signing_key;
    let mut merged_manifest_ids = Vec::new();
    for (name, chunks) in merged_manifests.values() {
        let compression = Compression::None;
        let mut manifest = FileManifest {
            name: name.clone(),
            size: 0,
            chunks: chunks.clone(),
            content_type: None,
            compression: compression.as_str().to_string(),
            merkle_root: Some(FileManifest::merkle_root(chunks)),
            created_by: Some(author.to_string()),
            created_at: Some(timestamp),
            signature: None,
        };

        let mut unsigned = manifest.clone();
        unsigned.signature = None;
        let canonical = metadata::serialize_for_signing(&unsigned);
        let sig = signing_key.sign(&canonical);
        manifest.signature = Some(hex::encode(sig.to_bytes()));

        let encoded = metadata::serialize(&manifest, &fmt);
        let hash = blake3::hash(&encoded);
        store.put_chunk(&crate::chunker::Chunk {
            hash,
            data: encoded,
            offset: 0,
        })?;
        merged_manifest_ids.push(hash.to_hex().to_string());
    }
    merged_manifest_ids.sort();

    // Create merge commit
    let public_key_hex = hex::encode(keys.verifying_key.to_bytes());
    let key_id = keychain::get_current_key_id(&shard_dir.join("keys")).ok();
    let parents = vec![current_id.clone(), source_id.clone()];
    let mut commit = Commit {
        commit_id: String::new(),
        parents,
        manifests: merged_manifest_ids,
        author: author.to_string(),
        message: message.to_string(),
        timestamp,
        public_key: Some(public_key_hex),
        signature: None,
        key_id,
    };

    let json_unsigned = metadata::serialize_for_signing(&commit);
    let signature = signing_key.sign(&json_unsigned);
    commit.signature = Some(hex::encode(signature.to_bytes()));

    let encoded = metadata::serialize(&commit, &fmt);
    let hash = blake3::hash(&encoded);
    store.put_chunk(&crate::chunker::Chunk {
        hash,
        data: encoded,
        offset: 0,
    })?;

    let merge_commit_id = hash.to_hex().to_string();

    // Cycle detection on merge
    if has_dag_cycle(&store, &commit.parents, &merge_commit_id)? {
        anyhow::bail!(
            "Cycle detected in merge: commit {} is already an ancestor of one or more parents",
            merge_commit_id
        );
    }

    // Update HEAD and branch ref
    if let Some(ref branch_name) = current_branch {
        branch::update_branch_ref(&shard_dir, branch_name, &merge_commit_id)?;
        branch::set_head_branch(&shard_dir, branch_name)?;
    } else {
        branch::set_head_commit(&shard_dir, &merge_commit_id)?;
    }

    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "commit_id": merge_commit_id,
            "message": message,
        }))?);
    } else {
        info!("Merge commit {} ({})", merge_commit_id, message);
    }
    Ok(())
}

pub fn tag_list(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let tags = load_tags(&shard_dir)?;
    if tags.is_empty() {
        info!("No tags.");
    } else {
        for (name, commit_id) in &tags {
            info!("{} -> {}", name, commit_id);
        }
    }
    Ok(())
}

fn collect_reachable(
    store: &Store,
    commit_id: &str,
    seen_commits: &mut std::collections::HashSet<String>,
    reachable: &mut std::collections::HashSet<String>,
) -> Result<()> {
    if !seen_commits.insert(commit_id.to_string()) {
        return Ok(());
    }

    reachable.insert(commit_id.to_string());

    let commit = match load_commit(store, commit_id) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    for manifest_id in &commit.manifests {
        reachable.insert(manifest_id.clone());

        if let Ok(data) = store.get_chunk(manifest_id) {
            if let Ok(manifest) = metadata::deserialize::<FileManifest>(&data) {
                for chunk_id in &manifest.chunks {
                    reachable.insert(chunk_id.clone());
                }
            }
        }
    }

    for parent_id in &commit.parents {
        collect_reachable(store, parent_id, seen_commits, reachable)?;
    }

    Ok(())
}

pub fn prune(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let config = load_config(&shard_dir)?;
    let fmt = MetadataFormat::from_config(&config);

    let store = Store::open(&shard_dir)?;
    let mut reachable: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Walk from HEAD commit (and all branch tips)
    let (_, head_commit) = branch::resolve_head(&shard_dir)?;
    if let Some(ref head) = head_commit {
        collect_reachable(
            &store,
            head,
            &mut std::collections::HashSet::new(),
            &mut reachable,
        )?;
    }

    // Also walk from all branch refs (in case HEAD is detached from any branch)
    if let Ok(branches) = branch::list_branches(&shard_dir) {
        for (_, commit_id) in branches.1 {
            collect_reachable(
                &store,
                &commit_id,
                &mut std::collections::HashSet::new(),
                &mut reachable,
            )?;
        }
    }

    // 2. Walk from tags
    let tags = load_tags(&shard_dir)?;
    for commit_id in tags.values() {
        collect_reachable(
            &store,
            commit_id,
            &mut std::collections::HashSet::new(),
            &mut reachable,
        )?;
    }

    // 3. Walk from index (staged files)
    let index = Index::load(&shard_dir.join("index"), &fmt)?;
    for manifest in index.files.values() {
        let json = metadata::serialize(manifest, &fmt);
        let hash = blake3::hash(&json);
        let hash_hex = hash.to_hex().to_string();
        reachable.insert(hash_hex);
        for chunk_hash in &manifest.chunks {
            reachable.insert(chunk_hash.clone());
        }
    }

    // 4. Scan objects and remove unreachable
    let all_chunks = store.iter_chunks()?;
    let mut pruned = 0u64;
    let mut kept = 0u64;
    for (hash_hex, full_path) in &all_chunks {
        if !reachable.contains(hash_hex) {
            store.delete_chunk(hash_hex, Some(full_path))?;
            pruned += 1;
        } else {
            kept += 1;
        }
    }

    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "pruned": pruned,
            "remaining": kept,
        }))?);
    } else {
        info!("Pruned {} objects. {} objects remain.", pruned, kept);
    }
    Ok(())
}

pub fn peer_add(path: &Path, multiaddr: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    // Validate multiaddr format
    if multiaddr.is_empty() || multiaddr.parse::<shard_net::libp2p::Multiaddr>().is_err() {
        anyhow::bail!("invalid multiaddr '{}' (must be a valid libp2p multiaddr, e.g. /ip4/1.2.3.4/tcp/5678/p2p/...)", multiaddr);
    }

    let peers_path = shard_dir.join("peers.json");
    let mut peers: Vec<String> = if peers_path.exists() {
        let data = fs::read(&peers_path)?;
        serde_json::from_slice(&data)?
    } else {
        Vec::new()
    };

    let added = if !peers.contains(&multiaddr.to_string()) {
        peers.push(multiaddr.to_string());
        let data = serde_json::to_vec(&peers)?;
        fs::write(peers_path, data)?;
        true
    } else {
        false
    };

    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "multiaddr": multiaddr,
            "added": added,
        }))?);
    } else if added {
        info!("Added peer: {}", multiaddr);
    } else {
        info!("Peer already exists: {}", multiaddr);
    }

    Ok(())
}

fn load_peers(shard_dir: &Path) -> Result<Vec<String>> {
    let peers_path = shard_dir.join("peers.json");
    if peers_path.exists() {
        let data = fs::read(peers_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(Vec::new())
    }
}

fn authorized_keys_path(shard_dir: &Path) -> std::path::PathBuf {
    shard_dir.join("authorized_keys")
}

fn load_authorized_keys(shard_dir: &Path) -> Result<Vec<ed25519_dalek::VerifyingKey>> {
    let path = authorized_keys_path(shard_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let mut keys = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bytes = hex::decode(line)?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key length in authorized_keys"))?;
        keys.push(ed25519_dalek::VerifyingKey::from_bytes(&arr)?);
    }
    Ok(keys)
}

pub fn add_authorized_key(shard_dir: &Path, public_key_hex: &str) -> Result<()> {
    // Validate the key
    let bytes = hex::decode(public_key_hex)?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Public key must be 32 bytes (64 hex chars)"))?;
    let _pk = ed25519_dalek::VerifyingKey::from_bytes(&arr)?;

    let path = authorized_keys_path(shard_dir);
    let mut content = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    // Check if key already exists
    if content.lines().any(|l| l.trim() == public_key_hex) {
        info!("Key already authorized");
        return Ok(());
    }
    content.push_str(public_key_hex);
    content.push('\n');
    fs::write(&path, content)?;
    info!("Authorized key added");
    Ok(())
}

pub fn backup(path: &Path, output: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let file = fs::File::create(output)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all(".", &shard_dir)?;
    archive.finish()?;
    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "path": output.to_string_lossy(),
        }))?);
    } else {
        info!("Backup created: {}", output.display());
    }
    Ok(())
}

pub fn export(path: &Path, commit_id: &str, output_dir: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let store = Store::open(&shard_dir)?;
    let cipher = maybe_load_cipher(&shard_dir)?;
    let commit = load_commit(&store, commit_id)?;
    let mut files = Vec::new();
    for manifest_id in &commit.manifests {
        let data = store.get_chunk(manifest_id)?;
        let manifest: FileManifest = metadata::deserialize(&data)?;
        let compression = manifest.compression.parse::<Compression>()?;
        if !json {
            info!("Exporting file: {}", manifest.name);
        }
        let mut file_data = Vec::new();
        for chunk_id in &manifest.chunks {
            let chunk_data = store.get_chunk(chunk_id)?;
            let decrypted = match &cipher {
                Some(c) => c.decrypt(&chunk_data)?,
                None => chunk_data,
            };
            let decompressed = compression.decompress(&decrypted)?;
            file_data.extend_from_slice(&decompressed);
        }
        let out_path = output_dir.join(&manifest.name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&out_path, file_data)?;
        if !json {
            info!("  -> {}", out_path.display());
        }
        files.push(manifest.name);
    }
    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit_id": commit_id,
                "files": files,
                "output_dir": output_dir.to_string_lossy(),
            }))?
        );
    } else {
        info!("Export complete.");
    }
    Ok(())
}

pub fn import(path: &Path, source_dir: &Path, message: &str, author: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    // Walk files in source_dir
    let config = load_config(&shard_dir)?;
    let compression: Compression = config
        .get("compression")
        .map(|s| s.as_str())
        .unwrap_or("zstd")
        .parse()?;
    let chunker_mode = chunker::ChunkerMode::from_config(&config);
    let fmt = MetadataFormat::from_config(&config);
    let store = Store::open(&shard_dir)?;
    let cipher = maybe_load_cipher(&shard_dir)?;
    let mut index = Index::load(&shard_dir.join("index"), &fmt)?;
    if !source_dir.is_dir() {
        anyhow::bail!("Source must be a directory");
    }
    for entry in walkdir::WalkDir::new(source_dir)
        .into_iter()
        .filter_entry(|e| {
            e.file_name()
                .to_str()
                .map(|s| !s.starts_with('.'))
                .unwrap_or(false)
        })
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            add_file(
                path,
                entry.path(),
                &store,
                &mut index,
                &compression,
                &chunker_mode,
                cipher.as_ref(),
                json,
            )?;
        }
    }
    index.save(&shard_dir.join("index"), &fmt)?;
    // Auto-commit
    if !index.files.is_empty() {
        commit(path, message, author, json)?;
    } else if json {
        info!("{}", serde_json::to_string(&serde_json::json!({"status": "no files found"}))?);
    } else {
        info!("No files found to import.");
    }
    Ok(())
}

pub fn restore(path: &Path, backup_file: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!(
            "Repository already exists — remove .shard first or use a different directory"
        );
    }
    let file = fs::File::open(backup_file)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(path)?;
    // Verify the result
    if !path.join(".shard").exists() {
        anyhow::bail!("Backup does not contain a valid .shard directory");
    }
    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "backup": backup_file.to_string_lossy(),
        }))?);
    } else {
        info!("Restored from {}", backup_file.display());
    }
    Ok(())
}

struct RepoProvider {
    store: Store,
    shard_dir: std::path::PathBuf,
}

impl shard_net::p2p::ShardContentProvider for RepoProvider {
    fn get_manifest(&self, id: &str) -> Option<Vec<u8>> {
        self.store.get_chunk(id).ok()
    }
    fn get_chunk(&self, id: &str) -> Option<Vec<u8>> {
        self.store.get_chunk(id).ok()
    }
    fn put_chunk(&mut self, id: &str, data: &[u8]) -> bool {
        let hash = blake3::hash(data);
        let hex = hash.to_hex().to_string();
        if hex != id {
            return false;
        }
        self.store
            .put_chunk(&crate::chunker::Chunk {
                hash,
                data: data.to_vec(),
                offset: 0,
            })
            .is_ok()
    }
    fn verify_auth(&self, public_key: &[u8], nonce: &[u8], signature: &[u8]) -> bool {
        use ed25519_dalek::Verifier;
        let pk_bytes: [u8; 32] = match public_key.try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let pk = match ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes) {
            Ok(k) => k,
            Err(_) => return false,
        };
        let sig_bytes: [u8; 64] = match signature.try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        if pk.verify(nonce, &sig).is_err() {
            return false;
        }
        // Check authorized_keys if the file exists
        if let Ok(keys) = load_authorized_keys(&self.shard_dir) {
            if !keys.is_empty() {
                return keys.contains(&pk);
            }
        }
        true
    }
    fn repo_public_key(&self) -> Option<Vec<u8>> {
        let keys = shard_crypto::KeyPair::load(&self.shard_dir.join("keys")).ok()?;
        Some(keys.verifying_key.to_bytes().to_vec())
    }
}

pub async fn share(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let mut node = shard_net::p2p::Node::new().await?;

    // Bootstrap from peers
    let peers = load_peers(&shard_dir)?;
    for peer in peers {
        if let Ok(addr) = peer.parse::<shard_net::libp2p::Multiaddr>() {
            let _ = node.swarm.dial(addr);
        }
    }

    node.listen("/ip4/0.0.0.0/tcp/0").await?;

    let store = Store::open(&shard_dir)?;
    let provider = RepoProvider {
        store,
        shard_dir: shard_dir.clone(),
    };
    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "status": "sharing",
            "peer_id": node.local_peer_id().to_string(),
        }))?);
    } else {
        info!("Sharing repository...");
    }
    node.run(provider).await;

    Ok(())
}

/// Start a circuit relay v2 server for NAT traversal.
/// Listens on the given address and forwards traffic between peers.
pub async fn relay(listen_addr: &str, json: bool) -> Result<()> {
    let mut node = shard_net::p2p::Node::new().await?;
    node.listen(listen_addr).await?;
    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "status": "relay active",
            "listen": listen_addr,
            "peer_id": node.local_peer_id().to_string(),
        }))?);
    } else {
        info!("Relay server active on {}", listen_addr);
        info!("Peer ID: {}", node.local_peer_id());
        info!("Ready to accept circuit relay v2 reservations");
    }
    node.run(EmptyProvider).await;
    Ok(())
}

/// Minimal provider for relay-only mode (no repo content needed).
struct EmptyProvider;
impl shard_net::p2p::ShardContentProvider for EmptyProvider {
    fn get_manifest(&self, _id: &str) -> Option<Vec<u8>> {
        None
    }
    fn get_chunk(&self, _id: &str) -> Option<Vec<u8>> {
        None
    }
    fn put_chunk(&mut self, _id: &str, _data: &[u8]) -> bool {
        false
    }
    fn verify_auth(&self, _public_key: &[u8], _nonce: &[u8], _signature: &[u8]) -> bool {
        false
    }
    fn repo_public_key(&self) -> Option<Vec<u8>> {
        None
    }
}

pub async fn sync(path: &Path, _json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let config = load_config(&shard_dir)?;
    let repo_id = config
        .get("repo_id")
        .ok_or_else(|| anyhow::anyhow!("No repo_id in config. Run `shard init` to create one."))?;
    let topic_str = format!("/shard/repo/{}", repo_id);
    let topic = shard_net::libp2p::gossipsub::IdentTopic::new(topic_str);

    let mut node = shard_net::p2p::Node::new().await?;
    node.subscribe(&topic)?;
    node.listen("/ip4/0.0.0.0/tcp/0").await?;

    // Bootstrap from configured peers
    let peers = load_peers(&shard_dir)?;
    for peer in peers {
        if let Ok(addr) = peer.parse::<shard_net::libp2p::Multiaddr>() {
            let _ = node.swarm.dial(addr);
        }
    }

    let head_commit = branch::resolve_head(&shard_dir)?.1;

    // Initial announce (may fail with InsufficientPeers if no peers yet)
    if let Some(ref head) = head_commit {
        let msg = format!("announce:{}", head);
        match node.publish(&topic, msg.as_bytes()) {
            Ok(_) => {
                if !_json {
                    info!("Announced commit {} on sync topic", head)
                }
            }
            Err(e) => error!("Initial announce (will retry): {}", e),
        }
    } else if !_json {
        info!("No commits to announce");
    }

    if !_json {
        info!("Syncing on topic with peer id: {}", node.local_peer_id());
    }
    let _ = std::io::stdout().flush();

    let store = Store::open(&shard_dir)?;
    let mut provider = RepoProvider {
        store,
        shard_dir: shard_dir.clone(),
    };

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut address_book: HashMap<shard_net::libp2p::PeerId, Vec<shard_net::libp2p::Multiaddr>> =
        HashMap::new();
    let path_buf = path.to_path_buf();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("\nSync shutting down...");
                break Ok(());
            }
            _ = interval.tick() => {
                if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                    let msg = format!("announce:{}", head);
                    match node.publish(&topic, msg.as_bytes()) {
                        Ok(_) => info!("Re-announced commit {} on sync topic", head),
                        Err(e) => error!("Re-announce failed: {}", e),
                    }
                }
            }
            event = node.swarm.select_next_some() => {
                match event {
                    shard_net::libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Listening on {address:?}");
                        let _ = std::io::stdout().flush();
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(
                            shard_net::libp2p::mdns::Event::Discovered(list),
                        ),
                    ) => {
                        for (peer_id, multiaddr) in list {
                            info!("mDNS discovered: {peer_id} {multiaddr}");
                            address_book.entry(peer_id).or_default().push(multiaddr.clone());
                            node.swarm
                                .behaviour_mut()
                                .gossipsub
                                .add_explicit_peer(&peer_id);
                            node.swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, multiaddr);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(shard_net::libp2p::mdns::Event::Expired(
                            list,
                        )),
                    ) => {
                        for (peer_id, _multiaddr) in list {
                            info!("mDNS expired: {peer_id}");
                            node.swarm
                                .behaviour_mut()
                                .gossipsub
                                .remove_explicit_peer(&peer_id);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Gossipsub(
                            shard_net::libp2p::gossipsub::Event::Message {
                                propagation_source,
                                message,
                                ..
                            },
                        ),
                    ) => {
                        if let Ok(text) = String::from_utf8(message.data.clone()) {
                            if let Some(commit_id) = text.strip_prefix("announce:") {
                                info!(
                                    "Peer {} announced commit: {}",
                                    propagation_source, commit_id
                                );
                                let peer = propagation_source;
                                let commit_id_owned = commit_id.to_string();
                                // Reply with our HEAD if different (triggers peer to pull from us)
                                let our_head = branch::resolve_head(&shard_dir)?.1.unwrap_or_default();
                                if our_head != commit_id_owned {
                                    let msg = format!("announce:{}", our_head);
                                    let _ = node.publish(&topic, msg.as_bytes());
                                }
                                if let Some(addrs) = address_book.get(&peer) {
                                    if let Some(addr) = addrs.first() {
                                        let multiaddr_str = format!("{}/p2p/{}", addr, peer);
                                        let path_clone = path_buf.clone();
                                        tokio::spawn(async move {
                                            match pull(&path_clone, &multiaddr_str, &commit_id_owned, false).await {
                                                Ok(_) => info!("Auto-pulled commit {} from {}", commit_id_owned, peer),
                                                Err(e) => error!("Auto-pull failed for commit {} from {}: {}", commit_id_owned, peer, e),
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::Message { peer, message },
                        ),
                    ) => {
                        if let shard_net::libp2p::request_response::Message::Request {
                            request, channel, ..
                        } = message
                        {
                            info!("Received request from {}", peer);
                            node.serve_request(&peer, &mut provider, request, channel);
                        } else {
                            info!("Received Response from {}", peer);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::OutboundFailure {
                                peer, error, ..
                            },
                        ),
                    ) => {
                        error!("Outbound failure to {}: {:?}", peer, error);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::InboundFailure {
                                peer, error, ..
                            },
                        ),
                    ) => {
                        error!("Inbound failure from {}: {:?}", peer, error);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Identify(
                            shard_net::libp2p::identify::Event::Received { peer_id, info },
                        ),
                    ) => {
                        info!("Identify received from {}: {:?}", peer_id, info.listen_addrs);
                        for addr in info.listen_addrs {
                            address_book.entry(peer_id).or_default().push(addr);
                        }
                        let _ = std::io::stdout().flush();
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Identify(event),
                    ) => {
                        info!("Identify event: {:?}", event);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        info!("Connection established with {}", peer_id);
                        // Only store the address when we dialed (it's the peer's listen addr).
                        // For listener connections, send_back_addr is the ephemeral port — useless for dialing back.
                        if let shard_net::libp2p::core::ConnectedPoint::Dialer { address, .. } = &endpoint {
                            address_book.entry(peer_id).or_default().push(address.clone());
                        }
                        node.swarm
                            .behaviour_mut()
                            .gossipsub
                            .add_explicit_peer(&peer_id);
                        // Announce HEAD to the newly connected peer
                        if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                            let msg = format!("announce:{}", head);
                            let _ = node.publish(&topic, msg.as_bytes());
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Relay(event),
                    ) => {
                        info!("Relay event: {:?}", event);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Dcutr(event),
                    ) => {
                        info!("DCUtR event: {:?}", event);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Autonat(event),
                    ) => {
                        info!("AutoNAT event: {:?}", event);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::IncomingConnection {
                        local_addr,
                        send_back_addr,
                        ..
                    } => {
                        info!(
                            "Incoming connection from {} to {}",
                            send_back_addr, local_addr
                        );
                    }
                    e => {
                        info!("Event: {:?}", e);
                    }
                }
            }
        }
    }
}

pub async fn pull(path: &Path, peer: &str, commit_id: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    // pull can work on empty repo or existing one.

    if !shard_dir.exists() {
        init(path, "flat", "zstd", "fixed", None, false, false)?;
    }

    let store = Store::open(&shard_dir)?;
    let cipher = maybe_load_cipher(&shard_dir)?;

    let mut node = shard_net::p2p::Node::new().await?;

    // Parse peer multiaddr
    let multiaddr: shard_net::libp2p::Multiaddr = peer.parse()?;
    let peer_id = match multiaddr.iter().last() {
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
        _ => anyhow::bail!("Multiaddr must end with /p2p/<peer_id>"),
    };

    // 1. Get Commit (sequential — single request)
    if !json {
        info!("Pulling commit {} from {}...", commit_id, peer);
    }
    let commit_data = node
        .request_manifest(&multiaddr, peer_id, commit_id.to_string())
        .await?;
    let hash = blake3::hash(&commit_data);
    if hash.to_hex().to_string() != commit_id {
        anyhow::bail!("Commit hash mismatch");
    }
    let chunk = crate::chunker::Chunk {
        hash,
        data: commit_data.clone(),
        offset: 0,
    };
    store.put_chunk(&chunk)?;

    let commit: Commit = metadata::deserialize(&commit_data)?;
    if !json {
        info!("Got commit: {}", commit.message);
    }

    // Fetch key rotation records for this commit's key chain
    let keys_dir = shard_dir.join("keys");
    if let Some(kid) = &commit.key_id {
        if keys_dir.join("records").exists() {
            if let Ok(chain) = keychain::collect_rotation_chain(&keys_dir, kid) {
                let missing_rotations: Vec<&KeyRotation> = chain
                    .iter()
                    .filter(|r| store.get_chunk(&r.rotation_id).is_err())
                    .collect();
                if !missing_rotations.is_empty() {
                    if !json {
                        info!(
                            "Fetching {} key rotation records from peer...",
                            missing_rotations.len()
                        );
                    }
                    let rot_requests: Vec<(String, shard_net::protocol::ShardRequest)> =
                        missing_rotations
                            .iter()
                            .map(|r| {
                                (
                                    r.rotation_id.clone(),
                                    shard_net::protocol::ShardRequest::GetChunk(
                                        r.rotation_id.clone(),
                                    ),
                                )
                            })
                            .collect();
                    if let Ok(rot_results) = node
                        .request_parallel(&multiaddr, peer_id, rot_requests)
                        .await
                    {
                        for (rot_id, rot_data) in &rot_results {
                            let rh = blake3::hash(rot_data);
                            if rh.to_hex().to_string() != *rot_id {
                                info!("Key rotation record hash mismatch (expected {}, got {}) — skipping", rot_id, rh.to_hex());
                                continue;
                            }
                            store.put_chunk(&crate::chunker::Chunk {
                                hash: rh,
                                data: rot_data.clone(),
                                offset: 0,
                            })?;
                        }
                        if !json { info!("Key rotation records synced from peer."); }
                    }
                }
            }
        }
    }

    // Set repo_id from commit's public key so clones share the gossipsub topic
    if let Some(pk_hex) = &commit.public_key {
        let pk_bytes = hex::decode(pk_hex)?;
        let repo_id = blake3::hash(&pk_bytes).to_hex().to_string();
        let mut config = load_config(&shard_dir)?;
        config.insert("repo_id".to_string(), repo_id);
        save_config(&shard_dir, &config)?;
    }

    // 2. Fetch all manifests in parallel
    let manifest_requests: Vec<(String, shard_net::protocol::ShardRequest)> = commit
        .manifests
        .iter()
        .map(|id| {
            (
                id.clone(),
                shard_net::protocol::ShardRequest::GetManifest(id.clone()),
            )
        })
        .collect();
    let manifest_results = node
        .request_parallel(&multiaddr, peer_id, manifest_requests)
        .await?;

    let mut all_chunk_ids: Vec<String> = Vec::new();
    let mut file_manifests: Vec<FileManifest> = Vec::new();
    // Map chunk_id -> compression type for verification in step 3
    let mut chunk_compression: HashMap<String, String> = HashMap::new();

    for (manifest_id, manifest_data) in &manifest_results {
        let hash = blake3::hash(manifest_data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }
        let chunk = crate::chunker::Chunk {
            hash,
            data: manifest_data.clone(),
            offset: 0,
        };
        store.put_chunk(&chunk)?;
        let manifest: FileManifest = metadata::deserialize(manifest_data)?;
        if !json {
            info!(
                "Fetching file: {} (compression: {})",
                manifest.name, manifest.compression
            );
        }
        for cid in &manifest.chunks {
            chunk_compression.insert(cid.clone(), manifest.compression.clone());
        }
        all_chunk_ids.extend(manifest.chunks.clone());
        file_manifests.push(manifest);
    }

    // 3. Resume: recover from partial directory if previous transfer was interrupted
    let partial = partial::PartialTransfer::new(&shard_dir, commit_id)?;
    let partial_chunks = partial.list_chunks()?;
    let mut recovered = 0usize;
    for chunk_id in &partial_chunks {
        if let Ok(data) = partial.load_chunk(chunk_id) {
            let hash = blake3::hash(&data);
            if hash.to_hex().to_string() == *chunk_id {
                // Recovered chunk matches — save to store
                let chunk = crate::chunker::Chunk {
                    hash,
                    data: data.clone(),
                    offset: 0,
                };
                if store.put_chunk(&chunk).is_ok() {
                    recovered += 1;
                }
            } else {
                // Corrupted partial chunk — remove and re-fetch
                let _ = partial.remove_chunk(chunk_id);
            }
        }
    }
    if recovered > 0 {
        info!("Recovered {} chunks from partial transfer", recovered);
    }

    // 4. Fetch all missing chunks (not in store and not in partial)
    let needed_chunks: Vec<String> = all_chunk_ids
        .into_iter()
        .filter(|id| store.get_chunk(id).is_err())
        .collect();

    if !needed_chunks.is_empty() {
        if !json {
            info!("Fetching {} chunks...", needed_chunks.len());
        }
        let chunk_requests: Vec<(String, shard_net::protocol::ShardRequest)> = needed_chunks
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    shard_net::protocol::ShardRequest::GetChunk(id.clone()),
                )
            })
            .collect();
        let chunk_results = node
            .request_parallel(&multiaddr, peer_id, chunk_requests)
            .await?;
        for (chunk_id, chunk_data) in &chunk_results {
            // Determine compression from the manifest this chunk belongs to
            let compression: Compression = chunk_compression
                .get(chunk_id)
                .map(|s| s.as_str())
                .unwrap_or("none")
                .parse()?;
            // Decrypt (if private) then decompress to verify the content hash
            let decrypted = match &cipher {
                Some(c) => c.decrypt(chunk_data)?,
                None => chunk_data.clone(),
            };
            let decompressed = compression.decompress(&decrypted)?;
            let hash = blake3::hash(&decompressed);
            if hash.to_hex().to_string() != *chunk_id {
                anyhow::bail!("Chunk hash mismatch: {}", chunk_id);
            }
            // Store the data as received (encrypted for private repos)
            let chunk = crate::chunker::Chunk {
                hash,
                data: chunk_data.clone(),
                offset: 0,
            };
            store.put_chunk(&chunk)?;
            // Save to partial for resume support
            partial.save_chunk(chunk_id, chunk_data)?;
        }
    }

    // 5. Reconstruct all files
    for manifest in &file_manifests {
        let compression = manifest.compression.parse::<Compression>()?;
        let mut file_data = Vec::new();
        for chunk_id in &manifest.chunks {
            let stored = store.get_chunk(chunk_id)?;
            let decrypted = match &cipher {
                Some(c) => c.decrypt(&stored)?,
                None => stored,
            };
            let decompressed = compression.decompress(&decrypted)?;
            file_data.extend_from_slice(&decompressed);
        }
        fs::write(path.join(&manifest.name), file_data)?;
        if !json {
            info!(
                "Reconstructed file: {} ({} bytes)",
                manifest.name, manifest.size
            );
        }
    }

    // 6. Clean up partial transfer tracking
    partial.cleanup()?;

    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "status": "pull complete",
            "commit_id": commit_id,
        }))?);
    } else {
        info!("Pull complete.");
    }
    Ok(())
}

pub fn transfer_list(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let transfers = partial::list_incomplete_transfers(&shard_dir)?;
    if json {
        info!("{}", serde_json::to_string(&transfers)?);
    } else {
        if transfers.is_empty() {
            info!("No incomplete transfers.");
        } else {
            for t in &transfers {
                info!("Incomplete transfer: {}", t);
            }
        }
    }
    Ok(())
}

pub fn transfer_remove(path: &Path, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    partial::remove_transfer(&shard_dir, commit_id)?;
    info!("Removed transfer tracking for {}", commit_id);
    Ok(())
}

pub async fn push(path: &Path, peer: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let (_, head_id) = branch::resolve_head(&shard_dir)?;
    let head_id = head_id.ok_or_else(|| anyhow::anyhow!("No commits to push"))?;

    let store = Store::open(&shard_dir)?;

    // Collect all reachable objects
    let mut objects: std::collections::BTreeMap<String, Vec<u8>> =
        std::collections::BTreeMap::new();

    // Walk commits
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![head_id.clone()];
    while let Some(cid) = stack.pop() {
        if !seen.insert(cid.clone()) {
            continue;
        }
        if let Ok(data) = store.get_chunk(&cid) {
            objects.insert(cid, data.clone());
            if let Ok(commit) = metadata::deserialize::<Commit>(&data) {
                // Include key rotation records for this commit's key chain
                if let Some(kid) = &commit.key_id {
                    let keys_dir = shard_dir.join("keys");
                    if let Ok(chain) = keychain::collect_rotation_chain(&keys_dir, kid) {
                        for rot in &chain {
                            let rj = serde_json::to_vec(rot)?;
                            let rh = blake3::hash(&rj);
                            if !store.has_chunk(rh.to_hex().as_ref()) {
                                store.put_chunk(&crate::chunker::Chunk {
                                    hash: rh,
                                    data: rj.clone(),
                                    offset: 0,
                                })?;
                            }
                            objects.insert(rot.rotation_id.clone(), rj);
                        }
                    }
                }
                for mid in &commit.manifests {
                    if let Ok(manifest_data) = store.get_chunk(mid) {
                        objects.insert(mid.clone(), manifest_data.clone());
                        if let Ok(manifest) = metadata::deserialize::<FileManifest>(&manifest_data)
                        {
                            for cid in &manifest.chunks {
                                if let Ok(chunk_data) = store.get_chunk(cid) {
                                    objects.insert(cid.clone(), chunk_data);
                                }
                            }
                        }
                    }
                }
                for parent in &commit.parents {
                    stack.push(parent.clone());
                }
            }
        }
    }

    if !json {
        info!(
            "Pushing {} objects ({} bytes)...",
            objects.len(),
            objects.values().map(|v| v.len() as u64).sum::<u64>()
        );
    }

    // Connect and send all objects
    let mut node = shard_net::p2p::Node::new().await?;
    let multiaddr: shard_net::libp2p::Multiaddr = peer.parse()?;
    let peer_id = match multiaddr.iter().last() {
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
        _ => anyhow::bail!("Multiaddr must end with /p2p/<peer_id>"),
    };

    for (id, data) in &objects {
        node.request_put_chunk(&multiaddr, peer_id, id.clone(), data.clone())
            .await?;
    }

    if json {
        info!("{}", serde_json::to_string(&serde_json::json!({
            "status": "push complete",
            "objects": objects.len(),
        }))?);
    } else {
        info!("Push complete ({} objects).", objects.len());
    }
    Ok(())
}

/// Rotate the signing key: generates a new ed25519 keypair, archives the old
/// one, and persists a signed rotation record.
pub fn key_rotate(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let keys_dir = shard_dir.join("keys");
    let rotation = keychain::rotate_signing_key(&keys_dir)?;

    // Store rotation record as a content-addressed chunk in the DAG
    let store = Store::open(&shard_dir)?;
    let json = serde_json::to_vec(&rotation)?;
    let hash = blake3::hash(&json);
    if !store.has_chunk(hash.to_hex().as_ref()) {
        store.put_chunk(&crate::chunker::Chunk {
            hash,
            data: json,
            offset: 0,
        })?;
    }

    // Replicate all rotation records as chunks for P2P availability
    let rotations = keychain::load_rotations(&keys_dir)?;
    for rot in &rotations {
        let rj = serde_json::to_vec(rot)?;
        let rh = blake3::hash(&rj);
        if !store.has_chunk(rh.to_hex().as_ref()) {
            store.put_chunk(&crate::chunker::Chunk {
                hash: rh,
                data: rj,
                offset: 0,
            })?;
        }
    }

    info!(
        "Key rotated: {} -> {}",
        rotation.old_key_id, rotation.new_key_id
    );
    info!("Rotation record: {} (stored in DAG)", rotation.rotation_id);
    Ok(())
}

/// List all keys in the keychain with their validity info.
pub fn key_list(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let keys_dir = shard_dir.join("keys");
    let records = keychain::load_records(&keys_dir)?;
    let current_id = keychain::get_current_key_id(&keys_dir)?;

    if json {
        info!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "current": current_id,
                "records": &records,
            }))?
        );
    } else {
        info!("Current key: {}", current_id);
        info!("Key history:");
        for record in &records {
            let marker = if record.key_id == current_id {
                " (active)"
            } else {
                ""
            };
            info!(
                "  {}  created_at={}{}",
                record.key_id, record.created_at, marker
            );
            if let Some(prev) = &record.previous_key_id {
                info!("    previous: {}", prev);
            }
        }
    }
    Ok(())
}

/// Verify the integrity of the keychain: check every rotation's signature.
pub fn key_verify(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }
    let keys_dir = shard_dir.join("keys");
    let errors = keychain::verify_keychain(&keys_dir)?;

    if json {
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "verified": errors.is_empty(),
                "errors": errors,
            }))?
        );
    } else {
        if errors.is_empty() {
            info!("Keychain verification successful.");
        } else {
            for err in &errors {
                error!("Keychain error: {}", err);
            }
            anyhow::bail!("Keychain verification failed ({} errors).", errors.len());
        }
    }
    Ok(())
}
