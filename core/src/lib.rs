//! Core engine for **Shard** — a local-first, content-addressed, peer-to-peer
//! version control system designed for large machine learning artifacts.
//!
//! `shard-core` implements chunking (fixed-size and FastCDC content-defined), self-describing
//! `SHRD` v1 compression/encryption envelopes (Zstd/Zlib + AES-256-GCM with AAD),
//! Ed25519-signed commit DAGs, branching/merging, CRC32-checksummed WAL recovery,
//! per-repo dual read/write locking (`.shard.lock`), garbage collection, backup/restore,
//! HuggingFace dataset export/import, an authenticated Axum HTTP API, and libp2p sync.
//!
//! # Quickstart
//!
//! ```rust
//! use std::fs;
//!
//! # fn main() -> anyhow::Result<()> {
//! let dir = tempfile::tempdir()?;
//! let repo = dir.path();
//!
//! shard_core::init(repo, "flat", "zstd", "fixed", None, false, true)?;
//! fs::write(repo.join("model.bin"), b"tensor weights v1")?;
//! shard_core::add(repo, &repo.join("model.bin"), true)?;
//! let commit_id = shard_core::commit(repo, "initial weights", "Alice <alice@example.com>", true)?;
//!
//! shard_core::verify(repo, &commit_id, true)?;
//! let snap = shard_core::status_snapshot(repo)?;
//! assert_eq!(snap.head.as_deref(), Some(commit_id.as_str()));
//! # Ok(())
//! # }
//! ```

pub mod api;
pub mod auth;
pub mod backup;
pub mod branch;
pub mod chunk_codec;
pub mod chunker;
pub mod commit;
pub mod compression;
pub mod config;
pub mod encryption;
pub mod error;
pub mod fs_util;
pub mod gc;
pub mod hf;
mod inbound_pins;
pub mod index;
pub mod keychain;
pub mod manifest;
pub mod metadata;
pub mod metrics;
pub mod ops;
pub mod p2p_sync;
pub mod partial;
pub mod repo;
pub mod store;
pub mod wal;

use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(crate) static OP_QUEUES: once_cell::sync::Lazy<ops::RepoOpQueues> =
    once_cell::sync::Lazy::new(ops::RepoOpQueues::new);

pub(crate) fn with_write_op<T>(
    path: &Path,
    desc: &str,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let _guard = OP_QUEUES
        .get_or_create(path)
        .acquire(ops::OpKind::Write, desc.to_string())?;
    let trace_id = ops::generate_trace_id();
    ops::set_trace_id(&trace_id);
    let result = f();
    if let Err(ref e) = result {
        metrics::METRICS
            .errors_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ops::traced_warn(format!("op {} failed: {}", desc, e));
    }
    ops::set_trace_id("");
    result
}

pub(crate) fn with_read_op<T>(path: &Path, desc: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = OP_QUEUES
        .get_or_create(path)
        .acquire(ops::OpKind::Read, desc.to_string())?;
    let trace_id = ops::generate_trace_id();
    ops::set_trace_id(&trace_id);
    let result = f();
    if let Err(ref e) = result {
        metrics::METRICS
            .errors_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ops::traced_warn(format!("op {} failed: {}", desc, e));
    }
    ops::set_trace_id("");
    result
}

pub(crate) fn load_config_file_only(shard_dir: &Path) -> Result<BTreeMap<String, String>> {
    let config_path = shard_dir.join("config.json");
    if config_path.exists() {
        let data = fs::read(&config_path)?;
        Ok(metadata::deserialize(&data)?)
    } else {
        Ok(BTreeMap::new())
    }
}

pub(crate) fn load_config(shard_dir: &Path) -> Result<BTreeMap<String, String>> {
    config::load_config(shard_dir)
}

pub(crate) fn save_config(shard_dir: &Path, cfg: &BTreeMap<String, String>) -> Result<()> {
    repo::write_config_atomic(shard_dir, cfg)
}

pub(crate) fn load_tags(shard_dir: &Path) -> Result<BTreeMap<String, String>> {
    let tags_path = shard_dir.join("tags.json");
    if tags_path.exists() {
        let data = fs::read(&tags_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(BTreeMap::new())
    }
}

pub(crate) fn save_tags(shard_dir: &Path, tags: &BTreeMap<String, String>) -> Result<()> {
    let data = serde_json::to_string_pretty(tags)?;
    fs_util::atomic_write(&shard_dir.join("tags.json"), data.as_bytes())?;
    Ok(())
}

// Re-export top-level API for full backward compatibility
pub use auth::{
    add_authorized_key, cache_passphrase, list_authorized_keys, remove_authorized_key, unlock_repo,
};
pub use backup::{backup, export, import, restore};
pub use hf::{hf_pull, hf_push};
pub use p2p_sync::{peer_add, pull, push, relay, share, sync, transfer_list, transfer_remove};
pub use repo::{
    add, branch_create, branch_delete, branch_list, checkout, commit, config_get, config_set, diff,
    get_metrics, health, init, init_with_passphrase, key_list, key_rotate, key_verify, log_cmd,
    log_entries, merge, prune, recover, status, status_snapshot, tag_add, tag_list, verify,
    verify_with_options, LogEntry, StatusSnapshot,
};
