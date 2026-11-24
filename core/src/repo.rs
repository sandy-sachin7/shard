use crate::auth;
use crate::branch;
use crate::chunk_codec;
use crate::chunker::{self, Chunk};
use crate::commit::Commit;
use crate::compression::Compression;
use crate::config;
use crate::encryption;
use crate::error::ShardError;
use crate::fs_util::{atomic_write, safe_repo_join, validate_object_id};
use crate::gc;
use crate::index::Index;
use crate::keychain;
use crate::manifest::FileManifest;
use crate::metadata::{self, MetadataFormat};
use crate::store::Store;
use crate::wal;
use anyhow::Result;
use ed25519_dalek::{Signer, Verifier};
use serde::Serialize;
use shard_crypto::KeyPair;
use similar::TextDiff;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, warn};

pub fn init(
    path: &Path,
    backend: &str,
    compression_algo: &str,
    chunker_mode: &str,
    chunk_size: Option<u64>,
    is_private: bool,
    json: bool,
) -> Result<()> {
    init_with_passphrase(
        path,
        backend,
        compression_algo,
        chunker_mode,
        chunk_size,
        is_private,
        json,
        "",
    )
}

#[allow(clippy::too_many_arguments)]
pub fn init_with_passphrase(
    path: &Path,
    backend: &str,
    compression_algo: &str,
    chunker_mode: &str,
    chunk_size: Option<u64>,
    is_private: bool,
    json: bool,
    passphrase: &str,
) -> Result<()> {
    crate::metrics::METRICS
        .ops_init
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "init", || {
        let shard_dir = path.join(".shard");
        if shard_dir.exists() {
            return Err(ShardError::AlreadyInitialized(shard_dir.display().to_string()).into());
        }

        // Build and validate config BEFORE creating repo directories
        let mut config = BTreeMap::new();
        if is_private {
            config.insert("private".to_string(), "true".to_string());
        }
        config.insert("storage_backend".to_string(), backend.to_string());
        config.insert(
            "serialization_format".to_string(),
            MetadataFormat::Json.config_value().to_string(),
        );
        config.insert("compression".to_string(), compression_algo.to_string());
        config.insert("chunker_mode".to_string(), chunker_mode.to_string());
        match chunker_mode {
            "rabin" => {
                let cs = chunk_size.unwrap_or(4_194_304);
                let min = (cs / 4).max(1);
                let max = (cs * 2).min(chunker::MAX_CHUNK_SIZE as u64);
                config.insert("chunk_min".to_string(), min.to_string());
                config.insert("chunk_avg".to_string(), cs.to_string());
                config.insert("chunk_max".to_string(), max.to_string());
            }
            _ => {
                let cs = chunk_size.unwrap_or(4_194_304);
                config.insert("chunk_size".to_string(), cs.to_string());
            }
        }
        let cfg_errors = config::validate_config(&config);
        if !cfg_errors.is_empty() {
            return Err(ShardError::InvalidConfig(cfg_errors.join("; ")).into());
        }

        fs::create_dir_all(shard_dir.join("objects"))?;
        fs::create_dir_all(shard_dir.join("keys"))?;
        fs::create_dir_all(shard_dir.join("refs").join("heads"))?;
        branch::set_head_branch(&shard_dir, "main")?;

        let keys = KeyPair::generate();
        keys.save_with_passphrase(&shard_dir.join("keys"), passphrase)?;
        if !passphrase.is_empty() {
            auth::cache_passphrase(passphrase)?;
        }
        if let Some(global) = auth::global_keys_dir() {
            if !global.join("secret.key").exists() {
                fs::create_dir_all(&global).ok();
                let _ = keys.save_with_passphrase(&global, passphrase);
                let _ = keychain::init_keychain(&global);
            }
        }
        keychain::init_keychain(&shard_dir.join("keys"))?;

        let pubkey = fs::read(shard_dir.join("keys/public.key"))?;
        let repo_id = blake3::hash(&pubkey).to_hex().to_string();
        config.insert("repo_id".to_string(), repo_id);

        if is_private {
            let key = encryption::generate_repo_key();
            encryption::save_repo_key_with_passphrase(&shard_dir.join("keys"), &key, passphrase)?;
        }

        crate::save_config(&shard_dir, &config)?;

        let chunker_desc = if chunker_mode == "rabin" {
            format!(
                "rabin (avg {} bytes)",
                config
                    .get("chunk_avg")
                    .map(|s| s.as_str())
                    .unwrap_or("4 MiB")
            )
        } else {
            format!(
                "fixed ({} bytes)",
                config
                    .get("chunk_size")
                    .map(|s| s.as_str())
                    .unwrap_or("4 MiB")
            )
        };
        if json {
            println!(
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
            println!(
                "Initialized empty Shard repository in {} with {} storage (compression: {}, chunking: {})",
                shard_dir.display(),
                backend,
                compression_algo,
                chunker_desc,
            );
        }
        Ok(())
    })
}

pub(crate) fn relative_path(base_root: &Path, file_path: &Path) -> String {
    let repo = base_root
        .canonicalize()
        .unwrap_or_else(|_| base_root.to_path_buf());
    let file = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    file.strip_prefix(&repo)
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
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
pub(crate) fn add_file(
    base_root: &Path,
    file_path: &Path,
    store: &Store,
    index: &mut Index,
    compression: &Compression,
    chunker_mode: &chunker::ChunkerMode,
    cipher: Option<&encryption::RepoCipher>,
    json: bool,
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
    let want_encrypted = cipher.is_some();

    while let Some(chunk) = chunker.next_chunk()? {
        let hash = chunk.hash;
        let hash_hex = hash.to_hex().to_string();

        // Dedup hit check: skip re-compressing/re-encrypting when an existing
        // self-describing SHRD envelope matches our encryption security requirement.
        let mut need_write = true;
        let mut overwrite = false;
        if store.has_chunk(&hash_hex) {
            if let Ok(existing) = store.get_chunk(&hash_hex) {
                if let Some(hdr) = chunk_codec::parse_header(&existing) {
                    if hdr.encrypted == want_encrypted {
                        // Self-describing envelope already matches privacy mode; decode
                        // automatically reads its own compression flag — safe dedup hit!
                        need_write = false;
                    } else {
                        // Privacy mode changed (e.g. unencrypted -> encrypted): overwrite safely
                        overwrite = true;
                    }
                } else {
                    // Legacy blob without SHRD header: upgrade cleanly if encoding changed
                    overwrite = true;
                }
            }
        }

        if need_write {
            let stored_data =
                chunk_codec::encode_chunk(&hash_hex, &chunk.data, *compression, cipher)?;
            let stored = Chunk {
                hash,
                data: stored_data,
                offset: chunk.offset,
            };
            if overwrite {
                store.put_chunk_overwrite(&stored)?;
            } else {
                store.put_chunk(&stored)?;
            }
            crate::metrics::METRICS
                .chunks_written
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::metrics::METRICS.bytes_written.fetch_add(
                chunk.data.len() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        }

        chunk_hashes.push(hash_hex);
        total_size += chunk.data.len() as u64;
    }

    let name = relative_path(base_root, file_path);
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

    index.removed.retain(|r| r != &name);
    index.files.insert(name.clone(), manifest);
    if !json {
        println!("Added {} ({})", name, total_size);
    }
    Ok(())
}

pub fn recover(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_recover
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "recover", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        wal::recover(&shard_dir)?;
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"status": "recovery complete"}))?
            );
        } else {
            println!("Recovery complete.");
        }
        Ok(())
    })
}

pub fn health(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_health
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "health", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        let mut issues: Vec<String> = Vec::new();

        let objects_ok = shard_dir.join("objects").exists();
        if !objects_ok {
            issues.push("objects directory missing".to_string());
        }

        let config_ok = shard_dir.join("config.json").exists();
        if config_ok {
            match crate::load_config(&shard_dir) {
                Ok(c) => {
                    for err in config::validate_config(&c) {
                        issues.push(format!("config invalid: {}", err));
                    }
                }
                Err(e) => {
                    issues.push(format!("config.json corrupt: {}", e));
                }
            }
        } else {
            issues.push("config.json missing".to_string());
        }

        let keys_dir = shard_dir.join("keys");
        let keys_ok = keys_dir.join("secret.key").exists() && keys_dir.join("public.key").exists();
        if !keys_ok {
            issues.push("keys missing (secret.key and/or public.key)".to_string());
        }

        let wal_has_entries = shard_dir.join("wal.log").exists()
            && fs::metadata(shard_dir.join("wal.log"))
                .map(|m| m.len() > 0)
                .unwrap_or(false);

        let store_result = Store::open(&shard_dir);
        let object_count = match &store_result {
            Ok(store) => store.iter_chunks().map(|c| c.len() as u64).unwrap_or(0),
            Err(e) => {
                issues.push(format!("store error: {}", e));
                0
            }
        };

        let (branch, head) = branch::resolve_head(&shard_dir)
            .ok()
            .unwrap_or((None, None));
        let branch_count = branch::list_branches(&shard_dir)
            .map(|(_, b)| b.len() as u64)
            .unwrap_or(0);
        let tags_count = crate::load_tags(&shard_dir)
            .map(|t| t.len() as u64)
            .unwrap_or(0);
        let disk_usage = dir_size(&shard_dir);
        let key_valid = auth::load_keypair(&shard_dir).is_ok();
        let metrics_snapshot = crate::metrics::METRICS.snapshot();

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "status": if issues.is_empty() { "healthy" } else { "degraded" },
                    "shard_dir": shard_dir.display().to_string(),
                    "objects_ok": objects_ok,
                    "config_ok": config_ok,
                    "keys_ok": keys_ok && key_valid,
                    "wal_pending": wal_has_entries,
                    "object_count": object_count,
                    "current_branch": branch,
                    "head_commit": head,
                    "branch_count": branch_count,
                    "tag_count": tags_count,
                    "disk_usage_bytes": disk_usage,
                    "issues": issues,
                    "metrics": metrics_snapshot,
                }))?
            );
        } else {
            if issues.is_empty() {
                println!("Health: OK");
            } else {
                println!("Health: DEGRADED");
                for issue in &issues {
                    warn!("  - {}", issue);
                }
            }
            println!("  Objects: {}", object_count);
            println!("  Branches: {}, Tags: {}", branch_count, tags_count);
            println!("  Current branch: {:?}", branch);
            println!("  HEAD: {:?}", head);
            println!("  WAL pending: {}", wal_has_entries);
            println!("  Disk usage: {} bytes", disk_usage);
        }

        if !issues.is_empty() {
            anyhow::bail!("health check found {} issue(s)", issues.len());
        }
        Ok(())
    })
}

fn dir_size(path: &Path) -> u64 {
    ignore::WalkBuilder::new(path)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

pub fn get_metrics() -> crate::metrics::MetricsSnapshot {
    crate::metrics::METRICS.snapshot()
}

pub fn add(path: &Path, file_path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_add
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "add", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        wal::recover(&shard_dir)?;

        let config = crate::load_config(&shard_dir)?;
        let compression: Compression = config
            .get("compression")
            .map(|s| s.as_str())
            .unwrap_or("zstd")
            .parse()?;

        let chunker_mode = chunker::ChunkerMode::from_config(&config);
        let fmt = MetadataFormat::from_config(&config);

        let store = Store::open(&shard_dir)?;
        let mut index = Index::load(&shard_dir.join("index"), &fmt)?;
        let cipher = auth::maybe_load_cipher(&shard_dir)?;
        let include_hidden = config
            .get("hidden_files_enabled")
            .map(|s| s.as_str() == "true")
            .unwrap_or(false);

        let resolved_target = if file_path.is_relative() {
            path.join(file_path)
        } else {
            file_path.to_path_buf()
        };

        if resolved_target.is_dir() {
            let mut builder = ignore::WalkBuilder::new(&resolved_target);
            builder.hidden(!include_hidden);
            builder.add_custom_ignore_filename(".shardignore");
            builder.filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                name != ".shard" && name != ".git"
            });

            for entry in builder.build() {
                let entry = entry.map_err(|e| anyhow::anyhow!("{}", e))?;
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
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
                &resolved_target,
                &store,
                &mut index,
                &compression,
                &chunker_mode,
                cipher.as_ref(),
                json,
            )?;
        }

        store.flush()?;
        index.save(&shard_dir.join("index"), &fmt)?;
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"status": "added"}))?
            );
        }
        Ok(())
    })
}

pub fn commit(path: &Path, message: &str, author: &str, json: bool) -> Result<String> {
    crate::metrics::METRICS
        .ops_commit
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "commit", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        wal::recover(&shard_dir)?;
        let store = Store::open(&shard_dir)?;
        commit_with_store(path, &shard_dir, &store, message, author, json)
    })
}

/// Commit staged files using an already-opened `Store` (avoids Sled double-open lock errors
/// when invoked from `import()` under an existing write lock).
pub(crate) fn commit_with_store(
    _path: &Path,
    shard_dir: &Path,
    store: &Store,
    message: &str,
    author: &str,
    json: bool,
) -> Result<String> {
    let config = crate::load_config(shard_dir)?;
    let fmt = MetadataFormat::from_config(&config);

    let mut index = Index::load(&shard_dir.join("index"), &fmt)?;
    if index.files.is_empty() && index.removed.is_empty() {
        anyhow::bail!("nothing to commit (stage files with `shard add` first)");
    }

    let head_path = shard_dir.join("HEAD");
    let (current_branch, parent_id) = branch::resolve_head(shard_dir)?;

    // WAL: back up HEAD, active branch ref, and index before mutating
    let wal = wal::Wal::new(shard_dir);
    let head_backup = fs::read_to_string(&head_path).ok();
    let branch_ref_backup = current_branch.as_ref().and_then(|b| {
        let p = shard_dir.join("refs").join("heads").join(b);
        fs::read_to_string(p).ok()
    });
    let index_backup = fs::read(shard_dir.join("index")).unwrap_or_default();
    wal.append(&wal::WalEntry::CommitBegin {
        head_backup,
        branch_name: current_branch.clone(),
        branch_ref_backup,
        index_backup,
    })?;

    // 1. Carry forward unmodified manifests from parent commit (true VCS snapshot semantics)
    let mut snapshot_manifests: BTreeMap<String, String> = BTreeMap::new();
    if let Some(ref pid) = parent_id {
        let parent_commit = load_commit(store, pid)?;
        for mid in &parent_commit.manifests {
            let data = store.get_chunk(mid)?;
            let m: FileManifest = metadata::deserialize(&data)?;
            snapshot_manifests.insert(m.name, mid.clone());
        }
    }

    // Apply explicit staged removals
    for removed_name in &index.removed {
        snapshot_manifests.remove(removed_name);
    }

    // 2. Store newly staged manifests (signed) and overlay onto snapshot
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let keys = auth::load_keypair(shard_dir)?;
    let signing_key = keys.signing_key;

    for (name, manifest) in index.files.iter_mut() {
        manifest.created_by = Some(author.to_string());
        manifest.created_at = Some(timestamp);

        let mut unsigned = manifest.clone();
        unsigned.signature = None;
        let canonical = metadata::serialize_for_signing(&unsigned);
        let sig = signing_key.sign(&canonical);
        manifest.signature = Some(hex::encode(sig.to_bytes()));

        let encoded = metadata::serialize(manifest, &fmt);
        let hash = blake3::hash(&encoded);
        store.put_chunk(&Chunk {
            hash,
            data: encoded,
            offset: 0,
        })?;
        snapshot_manifests.insert(name.clone(), hash.to_hex().to_string());
    }

    let mut manifest_ids: Vec<String> = snapshot_manifests.into_values().collect();
    manifest_ids.sort();
    manifest_ids.dedup();

    // 3. Build & sign commit
    let mut parents = Vec::new();
    if let Some(pid) = parent_id {
        parents.push(pid);
    }

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

    let json_unsigned = metadata::serialize_for_signing(&commit);
    let signature = signing_key.sign(&json_unsigned);
    commit.signature = Some(hex::encode(signature.to_bytes()));

    let encoded = metadata::serialize(&commit, &fmt);
    let hash = blake3::hash(&encoded);
    store.put_chunk(&Chunk {
        hash,
        data: encoded,
        offset: 0,
    })?;
    // Flush store BEFORE updating refs/HEAD for crash durability
    store.flush()?;

    let commit_id = hash.to_hex().to_string();
    if has_dag_cycle(store, &commit.parents, &commit_id)? {
        anyhow::bail!(
            "Cycle detected: commit {} is already an ancestor of one or more parents",
            commit_id
        );
    }

    // 4. Update branch ref and HEAD atomically
    if let Some(ref branch_name) = current_branch {
        branch::update_branch_ref(shard_dir, branch_name, &commit_id)?;
        branch::set_head_branch(shard_dir, branch_name)?;
    } else {
        branch::set_head_commit(shard_dir, &commit_id)?;
    }

    // 5. Clear index atomically
    index.files.clear();
    index.removed.clear();
    index.save(&shard_dir.join("index"), &fmt)?;

    // WAL: mark commit complete
    wal.append(&wal::WalEntry::CommitEnd)?;
    wal.truncate()?;

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit_id": commit_id,
                "message": message,
            }))?
        );
    } else {
        println!("Committed {} ({})", commit_id, message);
    }
    Ok(commit_id)
}

pub fn verify(path: &Path, commit_id: &str, json: bool) -> Result<()> {
    verify_with_options(path, commit_id, json, false)
}

pub fn verify_with_options(
    path: &Path,
    commit_id: &str,
    json: bool,
    allow_unsigned: bool,
) -> Result<()> {
    crate::metrics::METRICS
        .ops_verify
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "verify", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        validate_object_id(commit_id)?;
        let store = Store::open(&shard_dir)?;
        let cipher = auth::maybe_load_cipher(&shard_dir)?;
        let commit_data = store.get_chunk(commit_id)?;

        let stored_hash = blake3::hash(&commit_data).to_hex().to_string();
        if stored_hash != commit_id {
            return Err(ShardError::HashMismatch {
                kind: "commit",
                id: commit_id.to_string(),
                expected: commit_id.to_string(),
                actual: stored_hash,
            }
            .into());
        }

        let commit: Commit = metadata::deserialize(&commit_data)?;
        let verified_vk =
            auth::verify_commit_signature(&shard_dir, commit_id, &commit, allow_unsigned, true)?;
        let sig_verified = verified_vk.is_some();

        if !json {
            if let Some(ref kid) = commit.key_id {
                if keychain::resolve_public_key(&shard_dir.join("keys"), kid).is_ok() {
                    println!("Keychain: key {} was active at commit time.", kid);
                }
            }
            if sig_verified {
                println!("Signature verified.");
            } else {
                println!("Warning: Commit is unsigned (--allow-unsigned enabled).");
            }
        }

        let trusted_keys = auth::load_trusted_keys(&shard_dir)?;
        let mut files_checked = 0u64;

        for manifest_id in &commit.manifests {
            validate_object_id(manifest_id)?;
            let manifest_data = store.get_chunk(manifest_id)?;
            let hash = blake3::hash(&manifest_data).to_hex().to_string();
            if hash != *manifest_id {
                return Err(ShardError::HashMismatch {
                    kind: "manifest",
                    id: manifest_id.clone(),
                    expected: manifest_id.clone(),
                    actual: hash,
                }
                .into());
            }

            let manifest: FileManifest = metadata::deserialize(&manifest_data)?;

            // Verify manifest signature against the commit's verified key or any trusted key
            // (carried-forward manifests from earlier commits may have been signed by an
            // earlier rotated key from the same trusted keychain).
            if let Some(sig_hex) = &manifest.signature {
                let mut unsigned = manifest.clone();
                unsigned.signature = None;
                let canonical = metadata::serialize_for_signing(&unsigned);
                let sig_bytes = hex::decode(sig_hex)?;
                let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
                    ShardError::SignatureVerificationFailed(
                        "invalid manifest signature length".into(),
                    )
                })?;
                let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);

                let ok_commit_key = verified_vk
                    .as_ref()
                    .map(|vk| vk.verify(&canonical, &sig).is_ok())
                    .unwrap_or(false);
                let ok_trusted = ok_commit_key
                    || trusted_keys
                        .iter()
                        .any(|vk| vk.verify(&canonical, &sig).is_ok());
                if !ok_trusted {
                    return Err(ShardError::SignatureVerificationFailed(format!(
                        "manifest signature verification failed for '{}'",
                        manifest.name
                    ))
                    .into());
                }
                if !json {
                    println!("  Manifest signature verified for: {}", manifest.name);
                }
            }

            let compression = manifest.compression.parse::<Compression>()?;
            if !json {
                println!(
                    "Verifying file: {} (compression: {})",
                    manifest.name, manifest.compression
                );
            }

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
                validate_object_id(chunk_id)?;
                let chunk_data = store.get_chunk(chunk_id)?;
                let _plaintext =
                    chunk_codec::decode_chunk(chunk_id, &chunk_data, compression, cipher.as_ref())?;
            }
            files_checked += 1;
        }

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "commit_id": commit_id,
                    "verified": true,
                    "signature_verified": sig_verified,
                    "files_checked": files_checked,
                }))?
            );
        } else {
            println!("Verification successful.");
        }
        Ok(())
    })
}

pub(crate) fn load_commit(store: &Store, commit_id: &str) -> Result<Commit> {
    validate_object_id(commit_id)?;
    let data = store.get_chunk(commit_id)?;
    let mut commit: Commit = metadata::deserialize(&data)?;
    commit.commit_id = commit_id.to_string();
    Ok(commit)
}

fn has_dag_cycle(store: &Store, parents: &[String], commit_id: &str) -> Result<bool> {
    let mut seen = HashSet::new();
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

pub fn log_entries(path: &Path, limit: Option<usize>) -> Result<Vec<LogEntry>> {
    crate::with_read_op(path, "log", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let store = Store::open(&shard_dir)?;
        let (_, head_commit) = branch::resolve_head(&shard_dir)?;
        let head = head_commit.ok_or_else(|| {
            anyhow::anyhow!("no commits yet (run `shard commit` after adding files)")
        })?;

        let mut entries: Vec<LogEntry> = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([head]);

        while let Some(cid) = queue.pop_front() {
            if let Some(max) = limit {
                if entries.len() >= max {
                    break;
                }
            }
            if !seen.insert(cid.clone()) {
                continue;
            }
            let commit = load_commit(&store, &cid)?;
            for parent in &commit.parents {
                queue.push_back(parent.clone());
            }
            entries.push(commit.into());
        }
        Ok(entries)
    })
}

pub fn log_cmd(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_log
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let entries = log_entries(path, None)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for entry in &entries {
            let ts = {
                let secs = entry.timestamp as i64;
                let tm = time::OffsetDateTime::from_unix_timestamp(secs)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
                tm.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| entry.timestamp.to_string())
            };
            println!("commit {}", entry.commit_id);
            if !entry.parents.is_empty() {
                println!("parents: {}", entry.parents.join(" "));
            }
            println!("author: {}", entry.author);
            println!("date:   {}", ts);
            println!();
            for line in entry.message.lines() {
                println!("    {}", line);
            }
            println!();
        }
    }
    Ok(())
}

/// Stream a file manifest's chunks directly to `dest_path` using a `BufWriter`,
/// verifying every chunk hash on the fly (bounded memory regardless of file size).
pub(crate) fn stream_manifest_to_file(
    store: &Store,
    manifest: &FileManifest,
    dest_root: &Path,
    cipher: Option<&encryption::RepoCipher>,
) -> Result<std::path::PathBuf> {
    let dest_path = safe_repo_join(dest_root, &manifest.name)?;
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let compression = manifest.compression.parse::<Compression>()?;

    // Stream into a same-dir temp file first, then atomically rename over `dest_path`
    // so interrupted checkouts/exports/pulls never leave half-written files.
    let parent = dest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("destination path has no parent"))?;
    let name = dest_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp_path = parent.join(format!(
        ".{}.shardtmp.{}.{}",
        name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ));

    let write_res = (|| -> Result<()> {
        let file = fs::File::create(&tmp_path)?;
        let mut writer = BufWriter::new(file);
        for chunk_id in &manifest.chunks {
            let stored = store.get_chunk(chunk_id)?;
            let plaintext = chunk_codec::decode_chunk(chunk_id, &stored, compression, cipher)?;
            writer.write_all(&plaintext)?;
        }
        writer.flush()?;
        writer.into_inner()?.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = fs::rename(&tmp_path, &dest_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(dest_path)
}

fn reconstruct_file_bounded(
    store: &Store,
    manifest: &FileManifest,
    cipher: Option<&encryption::RepoCipher>,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>> {
    if manifest.size as usize > max_bytes {
        return Ok(None);
    }
    let compression: Compression = manifest.compression.parse()?;
    let mut data = Vec::with_capacity(manifest.size as usize);
    for chunk_id in &manifest.chunks {
        let chunk_data = store.get_chunk(chunk_id)?;
        let plaintext = chunk_codec::decode_chunk(chunk_id, &chunk_data, compression, cipher)?;
        if data.len().saturating_add(plaintext.len()) > max_bytes {
            return Ok(None);
        }
        data.extend_from_slice(&plaintext);
    }
    Ok(Some(data))
}

pub fn diff(path: &Path, commit_a: &str, commit_b: Option<&str>, json: bool) -> Result<()> {
    const MAX_TEXT_DIFF_BYTES: usize = 16 * 1024 * 1024;
    crate::metrics::METRICS
        .ops_diff
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "diff", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        let store = Store::open(&shard_dir)?;
        let cipher = auth::maybe_load_cipher(&shard_dir)?;

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

        let files1 = load_commit_manifests(&store, &commit1)?;
        let files2 = load_commit_manifests(&store, &commit2)?;

        let mut all_names: Vec<&String> = files1.keys().chain(files2.keys()).collect();
        all_names.sort();
        all_names.dedup();

        let mut changes: Vec<serde_json::Value> = Vec::new();
        let mut diff_found = false;

        for name in all_names {
            match (files1.get(name), files2.get(name)) {
                (None, Some(manifest)) => {
                    diff_found = true;
                    match reconstruct_file_bounded(
                        &store,
                        manifest,
                        cipher.as_ref(),
                        MAX_TEXT_DIFF_BYTES,
                    )? {
                        Some(content) => {
                            let text = String::from_utf8_lossy(&content);
                            if json {
                                changes.push(serde_json::json!({
                                    "type": "added",
                                    "file": name,
                                    "lines": text.lines().collect::<Vec<_>>(),
                                }));
                            } else {
                                println!("--- /dev/null");
                                println!("+++ b/{}", name);
                                let lines: Vec<&str> = text.lines().collect();
                                println!("@@ -0,0 +1,{} @@", lines.len());
                                for line in &lines {
                                    println!("+{}", line);
                                }
                            }
                        }
                        None => {
                            if json {
                                changes.push(serde_json::json!({
                                    "type": "added",
                                    "file": name,
                                    "binary_or_large": true,
                                    "size": manifest.size,
                                }));
                            } else {
                                println!(
                                    "Binary/large file added: {} ({} bytes)",
                                    name, manifest.size
                                );
                            }
                        }
                    }
                }
                (Some(manifest), None) => {
                    diff_found = true;
                    match reconstruct_file_bounded(
                        &store,
                        manifest,
                        cipher.as_ref(),
                        MAX_TEXT_DIFF_BYTES,
                    )? {
                        Some(content) => {
                            let text = String::from_utf8_lossy(&content);
                            if json {
                                changes.push(serde_json::json!({
                                    "type": "removed",
                                    "file": name,
                                    "lines": text.lines().collect::<Vec<_>>(),
                                }));
                            } else {
                                println!("--- a/{}", name);
                                println!("+++ /dev/null");
                                let lines: Vec<&str> = text.lines().collect();
                                println!("@@ -1,{} +0,0 @@", lines.len());
                                for line in &lines {
                                    println!("-{}", line);
                                }
                            }
                        }
                        None => {
                            if json {
                                changes.push(serde_json::json!({
                                    "type": "removed",
                                    "file": name,
                                    "binary_or_large": true,
                                    "size": manifest.size,
                                }));
                            } else {
                                println!(
                                    "Binary/large file removed: {} ({} bytes)",
                                    name, manifest.size
                                );
                            }
                        }
                    }
                }
                (Some(ma), Some(mb)) => {
                    if ma.chunks == mb.chunks {
                        continue;
                    }
                    diff_found = true;
                    let ca =
                        reconstruct_file_bounded(&store, ma, cipher.as_ref(), MAX_TEXT_DIFF_BYTES)?;
                    let cb =
                        reconstruct_file_bounded(&store, mb, cipher.as_ref(), MAX_TEXT_DIFF_BYTES)?;
                    match (ca, cb) {
                        (Some(content_a), Some(content_b)) => {
                            let text_a = String::from_utf8_lossy(&content_a);
                            let text_b = String::from_utf8_lossy(&content_b);
                            if json {
                                changes.push(serde_json::json!({
                                    "type": "modified",
                                    "file": name,
                                    "old_lines": text_a.lines().collect::<Vec<_>>(),
                                    "new_lines": text_b.lines().collect::<Vec<_>>(),
                                }));
                            } else {
                                let diff = TextDiff::from_lines(text_a.as_ref(), text_b.as_ref());
                                let mut buf: Vec<u8> = Vec::new();
                                diff.unified_diff()
                                    .header(&format!("a/{}", name), &format!("b/{}", name))
                                    .to_writer(&mut buf)
                                    .map_err(|e| anyhow::anyhow!("diff output error: {}", e))?;
                                let output = String::from_utf8_lossy(&buf);
                                for line in output.lines() {
                                    println!("{}", line);
                                }
                            }
                        }
                        _ => {
                            if json {
                                changes.push(serde_json::json!({
                                    "type": "modified",
                                    "file": name,
                                    "binary_or_large": true,
                                    "old_size": ma.size,
                                    "new_size": mb.size,
                                }));
                            } else {
                                println!(
                                    "Binary/large files a/{} ({} bytes) and b/{} ({} bytes) differ",
                                    name, ma.size, name, mb.size
                                );
                            }
                        }
                    }
                }
                (None, None) => {}
            }
        }

        if !diff_found {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &serde_json::json!({"changes": changes, "message": "no differences"})
                    )?
                );
            } else {
                println!("No differences between the commits.");
            }
            return Ok(());
        }

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"changes": changes}))?
            );
        }
        Ok(())
    })
}

pub fn checkout(path: &Path, target: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_checkout
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "checkout", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        let store = Store::open(&shard_dir)?;
        let cipher = auth::maybe_load_cipher(&shard_dir)?;

        let is_branch = branch::validate_branch_name(target).is_ok()
            && shard_dir.join("refs").join("heads").join(target).is_file();

        let commit_id = if is_branch {
            fs::read_to_string(shard_dir.join("refs").join("heads").join(target))?
                .trim()
                .to_string()
        } else {
            target.to_string()
        };

        // Validate commit and every manifest path BEFORE touching HEAD or disk
        let commit = load_commit(&store, &commit_id)?;
        let mut manifests = Vec::with_capacity(commit.manifests.len());
        for manifest_id in &commit.manifests {
            validate_object_id(manifest_id)?;
            let data = store.get_chunk(manifest_id)?;
            let hash = blake3::hash(&data).to_hex().to_string();
            if hash != *manifest_id {
                anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
            }
            let manifest: FileManifest = metadata::deserialize(&data)?;
            // Pre-validate every destination path to fail fast before writing anything
            let _ = safe_repo_join(path, &manifest.name)?;
            manifests.push(manifest);
        }

        let mut files = Vec::with_capacity(manifests.len());
        for manifest in &manifests {
            if !json {
                println!(
                    "Checking out file: {} (compression: {})",
                    manifest.name, manifest.compression
                );
            }
            stream_manifest_to_file(&store, manifest, path, cipher.as_ref())?;
            if !json {
                println!("  -> {}", manifest.name);
            }
            files.push(manifest.name.clone());
        }

        // Only update HEAD AFTER every file has been verified and written successfully!
        if is_branch {
            branch::set_head_branch(&shard_dir, target)?;
        } else {
            branch::set_head_commit(&shard_dir, &commit_id)?;
        }

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "commit_id": commit_id,
                    "files": files,
                }))?
            );
        } else {
            println!("Checkout complete.");
        }
        Ok(())
    })
}

pub fn status(path: &Path, json: bool, porcelain: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_status
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "status", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        let config = crate::load_config(&shard_dir)?;
        let fmt = MetadataFormat::from_config(&config);

        let (current_branch, head_commit) = branch::resolve_head(&shard_dir)?;
        let commit_id = head_commit.clone();
        if !json && !porcelain {
            if let Some(ref cid) = commit_id {
                if let Some(ref branch) = current_branch {
                    println!("On branch: {}", branch);
                } else {
                    println!("HEAD detached at {}", cid);
                }
            } else {
                println!("No commits yet.");
            }
        }

        let index = Index::load(&shard_dir.join("index"), &fmt)?;
        let mut staged: Vec<String> = index.files.keys().cloned().collect();
        staged.sort();
        if !json && !porcelain {
            if staged.is_empty() {
                println!("Nothing staged.");
            } else {
                println!("\nStaged files:");
                for name in &staged {
                    println!("  {} (to be committed)", name);
                }
            }
        }

        let store = Store::open(&shard_dir)?;
        let mut deleted = Vec::new();
        let tracked_names: HashSet<String> = if let Some(head) = &commit_id {
            let mut names = HashSet::new();
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
            HashSet::new()
        };
        deleted.sort();

        if !json && !porcelain && !deleted.is_empty() {
            println!("\nDeleted files:");
            for name in &deleted {
                println!("  {} (deleted)", name);
            }
        }

        let mut untracked = Vec::new();
        let include_hidden = config
            .get("hidden_files_enabled")
            .map(|s| s.as_str() == "true")
            .unwrap_or(false);

        let mut builder = ignore::WalkBuilder::new(path);
        builder.hidden(!include_hidden);
        builder.add_custom_ignore_filename(".shardignore");

        for entry in builder.build().filter_map(|e| e.ok()) {
            if entry.depth() > 0 && entry.file_type().is_some_and(|ft| ft.is_file()) {
                let rel_path = entry.path().strip_prefix(path).unwrap_or(entry.path());
                let name = rel_path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");

                let is_internal = rel_path.components().any(|c| {
                    let seg = c.as_os_str().to_string_lossy();
                    seg == ".shard" || seg == ".git"
                });

                if !is_internal
                    && !index.files.contains_key(&name)
                    && !tracked_names.contains(&name)
                {
                    untracked.push(name);
                }
            }
        }
        untracked.sort();

        if !json && !porcelain && !untracked.is_empty() {
            println!("\nUntracked files:");
            for name in &untracked {
                println!("  {}", name);
            }
        }

        if porcelain {
            for name in &staged {
                println!("A  {}", name);
            }
            for name in &deleted {
                println!(" D {}", name);
            }
            for name in &untracked {
                println!("?? {}", name);
            }
            return Ok(());
        }

        if json {
            println!(
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
    })
}

pub fn config_get(path: &Path, key: Option<&str>) -> Result<()> {
    crate::metrics::METRICS
        .ops_config
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "config_get", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let config = crate::load_config(&shard_dir)?;
        if let Some(key) = key {
            match config.get(key) {
                Some(value) => println!("{} = {}", key, value),
                None => anyhow::bail!("config key not found: {}", key),
            }
        } else {
            for (k, v) in &config {
                println!("{} = {}", k, v);
            }
        }
        Ok(())
    })
}

pub fn config_set(path: &Path, key: &str, value: &str) -> Result<()> {
    crate::metrics::METRICS
        .ops_config
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "config_set", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let mut config = crate::load_config_file_only(&shard_dir)?;
        config.insert(key.to_string(), value.to_string());
        let errors = config::validate_config(&config);
        if !errors.is_empty() {
            return Err(ShardError::InvalidConfig(errors.join("; ")).into());
        }
        crate::save_config(&shard_dir, &config)?;
        println!("{} = {}", key, value);
        Ok(())
    })
}

pub fn tag_add(path: &Path, name: &str, commit_id: &str) -> Result<()> {
    crate::metrics::METRICS
        .ops_tag
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "tag_add", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let store = Store::open(&shard_dir)?;
        load_commit(&store, commit_id)?;
        let mut tags = crate::load_tags(&shard_dir)?;
        tags.insert(name.to_string(), commit_id.to_string());
        crate::save_tags(&shard_dir, &tags)?;
        println!("Tagged '{}' -> {}", name, commit_id);
        Ok(())
    })
}

pub fn tag_list(path: &Path) -> Result<()> {
    crate::metrics::METRICS
        .ops_tag
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "tag_list", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let tags = crate::load_tags(&shard_dir)?;
        if tags.is_empty() {
            println!("No tags.");
        } else {
            for (name, commit_id) in &tags {
                println!("{} -> {}", name, commit_id);
            }
        }
        Ok(())
    })
}

pub fn branch_create(path: &Path, name: &str, commit_id: Option<&str>) -> Result<()> {
    crate::with_write_op(path, "branch_create", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        branch::validate_branch_name(name)?;
        let id = match commit_id {
            Some(cid) => cid.to_string(),
            None => {
                let (_, head) = branch::resolve_head(&shard_dir)?;
                head.ok_or_else(|| anyhow::anyhow!("No commits yet — cannot create branch"))?
            }
        };
        let store = Store::open(&shard_dir)?;
        load_commit(&store, &id)?;
        branch::create_branch(&shard_dir, name, &id)
    })
}

pub fn branch_delete(path: &Path, name: &str) -> Result<()> {
    crate::with_write_op(path, "branch_delete", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        branch::delete_branch(&shard_dir, name)
    })
}

pub fn branch_list(path: &Path) -> Result<()> {
    crate::with_read_op(path, "branch_list", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let (current, branches) = branch::list_branches(&shard_dir)?;
        if branches.is_empty() {
            println!("No branches.");
            return Ok(());
        }
        for (name, commit_id) in &branches {
            let prefix = if current.as_deref() == Some(name) {
                "* "
            } else {
                "  "
            };
            println!(
                "{}{} ({})",
                prefix,
                name,
                &commit_id[..8.min(commit_id.len())]
            );
        }
        Ok(())
    })
}

fn load_commit_manifests(store: &Store, commit: &Commit) -> Result<HashMap<String, FileManifest>> {
    let mut files = HashMap::new();
    for mid in &commit.manifests {
        let data = store.get_chunk(mid)?;
        let m: FileManifest = metadata::deserialize(&data)?;
        files.insert(m.name.clone(), m);
    }
    Ok(files)
}

/// Find the Lowest Common Ancestor (merge base) of two commits in the DAG via BFS.
fn find_merge_base(store: &Store, commit_a: &str, commit_b: &str) -> Result<Option<String>> {
    let mut ancestors_a = HashSet::new();
    let mut queue_a = VecDeque::from([commit_a.to_string()]);
    while let Some(cid) = queue_a.pop_front() {
        if !ancestors_a.insert(cid.clone()) {
            continue;
        }
        if let Ok(c) = load_commit(store, &cid) {
            for p in c.parents {
                queue_a.push_back(p);
            }
        }
    }

    let mut visited_b = HashSet::new();
    let mut queue_b = VecDeque::from([commit_b.to_string()]);
    while let Some(cid) = queue_b.pop_front() {
        if !visited_b.insert(cid.clone()) {
            continue;
        }
        if ancestors_a.contains(&cid) {
            return Ok(Some(cid));
        }
        if let Ok(c) = load_commit(store, &cid) {
            for p in c.parents {
                queue_b.push_back(p);
            }
        }
    }
    Ok(None)
}

fn manifest_content_eq(a: &FileManifest, b: &FileManifest) -> bool {
    a.chunks == b.chunks && a.size == b.size
}

/// Perform an LCA-based 3-way merge of `branch` into current HEAD.
/// Preserves complete `FileManifest` metadata (compression, size, content_type, merkle_root)
/// and fails loudly on merge conflicts instead of silently clobbering files.
pub fn merge(path: &Path, branch: &str, message: &str, author: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_merge
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "merge", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        let store = Store::open(&shard_dir)?;
        let config = crate::load_config(&shard_dir)?;
        let fmt = MetadataFormat::from_config(&config);

        let (current_branch, current_id) = branch::resolve_head(&shard_dir)?;
        let current_id =
            current_id.ok_or_else(|| anyhow::anyhow!("No commits yet — nothing to merge into"))?;

        let source_id = branch::resolve_rev(&shard_dir, branch)?;
        if source_id == current_id {
            anyhow::bail!("Already up to date — source is the same commit as HEAD");
        }

        let current_commit = load_commit(&store, &current_id)?;
        let source_commit = load_commit(&store, &source_id)?;
        let base_id = find_merge_base(&store, &current_id, &source_id)?;

        let base_files = match base_id {
            Some(ref bid) => {
                let base_commit = load_commit(&store, bid)?;
                load_commit_manifests(&store, &base_commit)?
            }
            None => HashMap::new(),
        };
        let ours_files = load_commit_manifests(&store, &current_commit)?;
        let theirs_files = load_commit_manifests(&store, &source_commit)?;

        let mut all_paths: Vec<String> = base_files
            .keys()
            .chain(ours_files.keys())
            .chain(theirs_files.keys())
            .cloned()
            .collect();
        all_paths.sort();
        all_paths.dedup();

        let mut merged_manifests: BTreeMap<String, FileManifest> = BTreeMap::new();
        let mut conflicts: Vec<String> = Vec::new();

        for name in all_paths {
            let base = base_files.get(&name);
            let ours = ours_files.get(&name);
            let theirs = theirs_files.get(&name);

            match (base, ours, theirs) {
                // Both sides agree
                (_, Some(o), Some(t)) if manifest_content_eq(o, t) => {
                    merged_manifests.insert(name, o.clone());
                }
                (_, None, None) => {}
                // Ours unchanged from base -> take theirs
                (Some(b), Some(o), Some(t)) if manifest_content_eq(b, o) => {
                    merged_manifests.insert(name, t.clone());
                }
                (Some(b), Some(o), None) if manifest_content_eq(b, o) => {
                    // Deleted on theirs, unchanged on ours -> delete
                }
                (None, None, Some(t)) => {
                    // Added on theirs only
                    merged_manifests.insert(name, t.clone());
                }
                // Theirs unchanged from base -> keep ours
                (Some(b), Some(o), Some(t)) if manifest_content_eq(b, t) => {
                    merged_manifests.insert(name, o.clone());
                }
                (Some(b), None, Some(t)) if manifest_content_eq(b, t) => {
                    // Deleted on ours, unchanged on theirs -> delete
                }
                (None, Some(o), None) => {
                    // Added on ours only
                    merged_manifests.insert(name, o.clone());
                }
                // Both sides diverged -> conflict!
                _ => {
                    conflicts.push(name);
                }
            }
        }

        if !conflicts.is_empty() {
            return Err(ShardError::MergeConflict {
                count: conflicts.len(),
                files: conflicts.join(", "),
            }
            .into());
        }

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let keys = auth::load_keypair(&shard_dir)?;
        let signing_key = keys.signing_key;
        let mut merged_manifest_ids = Vec::with_capacity(merged_manifests.len());

        for mut manifest in merged_manifests.into_values() {
            // Preserve compression, size, content_type, chunks, merkle_root from the source manifest!
            if manifest.merkle_root.is_none() {
                manifest.merkle_root = Some(FileManifest::merkle_root(&manifest.chunks));
            }
            manifest.created_by = Some(author.to_string());
            manifest.created_at = Some(timestamp);
            manifest.signature = None;

            let canonical = metadata::serialize_for_signing(&manifest);
            let sig = signing_key.sign(&canonical);
            manifest.signature = Some(hex::encode(sig.to_bytes()));

            let encoded = metadata::serialize(&manifest, &fmt);
            let hash = blake3::hash(&encoded);
            store.put_chunk(&Chunk {
                hash,
                data: encoded,
                offset: 0,
            })?;
            merged_manifest_ids.push(hash.to_hex().to_string());
        }
        merged_manifest_ids.sort();

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
        store.put_chunk(&Chunk {
            hash,
            data: encoded,
            offset: 0,
        })?;
        store.flush()?;

        let merge_commit_id = hash.to_hex().to_string();
        if has_dag_cycle(&store, &commit.parents, &merge_commit_id)? {
            anyhow::bail!(
                "Cycle detected in merge: commit {} is already an ancestor of one or more parents",
                merge_commit_id
            );
        }

        if let Some(ref branch_name) = current_branch {
            branch::update_branch_ref(&shard_dir, branch_name, &merge_commit_id)?;
            branch::set_head_branch(&shard_dir, branch_name)?;
        } else {
            branch::set_head_commit(&shard_dir, &merge_commit_id)?;
        }

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "commit_id": merge_commit_id,
                    "message": message,
                }))?
            );
        } else {
            println!("Merge commit {} ({})", merge_commit_id, message);
        }
        Ok(())
    })
}

/// Remove unreachable objects — delegates directly to `gc::gc()` so there is a single
/// crash-safe, locked, fail-closed GC implementation.
pub fn prune(path: &Path, json: bool) -> Result<()> {
    gc::gc(path, json)
}

/// Rotate the signing key: generates a new ed25519 keypair, archives the old
/// one, and persists a signed rotation record. Supports passphrase-encrypted keys.
pub fn key_rotate(path: &Path) -> Result<()> {
    crate::metrics::METRICS
        .ops_key_rotate
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "key_rotate", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let keys_dir = shard_dir.join("keys");
        let pass = auth::get_passphrase();
        let pass_str = pass.as_deref().map(|s| s.as_str()).unwrap_or("");
        let rotation = keychain::rotate_signing_key_with_passphrase(&keys_dir, pass_str)?;

        let store = Store::open(&shard_dir)?;
        let rotations = keychain::load_rotations(&keys_dir)?;
        for rot in &rotations {
            let rj = serde_json::to_vec(rot)?;
            let rh = blake3::hash(&rj);
            if !store.has_chunk(rh.to_hex().as_ref()) {
                store.put_chunk(&Chunk {
                    hash: rh,
                    data: rj,
                    offset: 0,
                })?;
            }
        }
        store.flush()?;

        println!(
            "Key rotated: {} -> {}",
            rotation.old_key_id, rotation.new_key_id
        );
        println!("Rotation record: {} (stored in DAG)", rotation.rotation_id);
        Ok(())
    })
}

/// List all keys in the keychain with their validity info.
pub fn key_list(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_key_list
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "key_list", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let keys_dir = shard_dir.join("keys");
        let records = keychain::load_records(&keys_dir)?;
        let current_id = keychain::get_current_key_id(&keys_dir)?;

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "current": current_id,
                    "records": &records,
                }))?
            );
        } else {
            println!("Current key: {}", current_id);
            println!("Key history:");
            for record in &records {
                let marker = if record.key_id == current_id {
                    " (active)"
                } else {
                    ""
                };
                println!(
                    "  {}  created_at={}{}",
                    record.key_id, record.created_at, marker
                );
                if let Some(prev) = &record.previous_key_id {
                    println!("    previous: {}", prev);
                }
            }
        }
        Ok(())
    })
}

/// Verify the integrity of the keychain: check every rotation's signature.
/// Returns `Err` on verification failure in BOTH plain and `--json` modes so CLI exit code is non-zero.
pub fn key_verify(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_key_verify
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "key_verify", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let keys_dir = shard_dir.join("keys");
        let errors = keychain::verify_keychain(&keys_dir)?;

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "verified": errors.is_empty(),
                    "errors": errors,
                }))?
            );
            if !errors.is_empty() {
                anyhow::bail!("Keychain verification failed ({} errors).", errors.len());
            }
        } else if errors.is_empty() {
            println!("Keychain verification successful.");
        } else {
            for err in &errors {
                error!("Keychain error: {}", err);
            }
            anyhow::bail!("Keychain verification failed ({} errors).", errors.len());
        }
        Ok(())
    })
}

/// Atomically persist `config.json`.
pub(crate) fn write_config_atomic(
    shard_dir: &Path,
    config: &BTreeMap<String, String>,
) -> Result<()> {
    let fmt = MetadataFormat::from_config(config);
    let data = metadata::serialize(config, &fmt);
    atomic_write(&shard_dir.join("config.json"), &data)?;
    Ok(())
}
