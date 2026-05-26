use crate::branch;
use crate::commit::Commit;
use crate::index::Index;
use crate::manifest::FileManifest;
use crate::metadata::{self, MetadataFormat};
use crate::store::Store;
use std::collections::HashSet;

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::{info, warn};

static GC_RUNNING: AtomicBool = AtomicBool::new(false);

pub struct GcPolicy {
    pub interval_secs: u64,
    pub enabled: bool,
}

impl GcPolicy {
    pub fn from_config(shard_dir: &Path) -> Self {
        let cfg = crate::load_config(shard_dir).unwrap_or_default();
        let interval_secs = cfg
            .get("gc_interval_secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(3600);
        let enabled = cfg.get("gc_enabled").map(|s| s == "true").unwrap_or(false);
        Self {
            interval_secs,
            enabled,
        }
    }
}

pub fn collect_reachable(
    store: &Store,
    commit_id: &str,
    seen_commits: &mut HashSet<String>,
    reachable: &mut HashSet<String>,
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

fn load_commit(store: &Store, commit_id: &str) -> Result<Commit> {
    let data = store.get_chunk(commit_id)?;
    let mut commit: Commit = metadata::deserialize(&data)?;
    commit.commit_id = commit_id.to_string();
    Ok(commit)
}

pub fn gc(path: &Path, json: bool) -> Result<()> {
    if GC_RUNNING.swap(true, Ordering::Acquire) {
        anyhow::bail!("GC already in progress");
    }
    let result = gc_inner(path, json);
    GC_RUNNING.store(false, Ordering::Release);
    result
}

fn gc_inner(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_prune
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository");
    }

    let cfg = crate::load_config(&shard_dir)?;
    let fmt = MetadataFormat::from_config(&cfg);
    let store = Store::open(&shard_dir)?;
    let mut reachable: HashSet<String> = HashSet::new();

    let (_, head_commit) = branch::resolve_head(&shard_dir)?;
    if let Some(ref head) = head_commit {
        collect_reachable(&store, head, &mut HashSet::new(), &mut reachable)?;
    }

    if let Ok(branches) = branch::list_branches(&shard_dir) {
        for (_, commit_id) in branches.1 {
            collect_reachable(&store, &commit_id, &mut HashSet::new(), &mut reachable)?;
        }
    }

    let tags = crate::load_tags(&shard_dir)?;
    for commit_id in tags.values() {
        collect_reachable(&store, commit_id, &mut HashSet::new(), &mut reachable)?;
    }

    let index = Index::load(&shard_dir.join("index"), &fmt)?;
    for manifest in index.files.values() {
        let json_bytes = metadata::serialize(manifest, &fmt);
        let hash = blake3::hash(&json_bytes);
        reachable.insert(hash.to_hex().to_string());
        for chunk_hash in &manifest.chunks {
            reachable.insert(chunk_hash.clone());
        }
    }

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
        info!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "pruned": pruned,
                "remaining": kept,
            }))?
        );
    } else {
        info!("GC pruned {} objects. {} objects remain.", pruned, kept);
    }
    Ok(())
}

pub async fn gc_loop(path: PathBuf, policy: GcPolicy) {
    if !policy.enabled {
        info!("Auto-GC disabled (set gc_enabled=true in config)");
        return;
    }
    info!("Auto-GC started, interval {}s", policy.interval_secs);
    let mut interval = tokio::time::interval(Duration::from_secs(policy.interval_secs));
    interval.tick().await;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let start = Instant::now();
                info!("Auto-GC: starting");
                match gc(&path, false) {
                    Ok(()) => info!("Auto-GC: completed in {:?}", start.elapsed()),
                    Err(e) => warn!("Auto-GC: error: {}", e),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Auto-GC stopped");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_gc_policy_default() {
        let dir = tempdir().unwrap();
        let policy = GcPolicy::from_config(dir.path());
        assert_eq!(policy.interval_secs, 3600);
        assert!(!policy.enabled);
    }

    #[test]
    fn test_collect_reachable_empty() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".shard/objects")).unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut reachable = HashSet::new();
        let result = collect_reachable(&store, "nonexistent", &mut HashSet::new(), &mut reachable);
        assert!(result.is_ok());
    }
}
