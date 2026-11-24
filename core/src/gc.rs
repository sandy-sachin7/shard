use crate::branch;
use crate::commit::Commit;
use crate::index::Index;
use crate::keychain;
use crate::manifest::FileManifest;
use crate::metadata::{self, MetadataFormat};
use crate::partial;
use crate::store::Store;
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::{info, warn};

static GC_RUNNING: AtomicBool = AtomicBool::new(false);

/// RAII guard that guarantees `GC_RUNNING` is cleared even if GC panics.
struct GcRunningGuard;

impl GcRunningGuard {
    fn acquire() -> Result<Self> {
        if GC_RUNNING.swap(true, Ordering::Acquire) {
            anyhow::bail!("GC already in progress");
        }
        Ok(Self)
    }
}

impl Drop for GcRunningGuard {
    fn drop(&mut self) {
        GC_RUNNING.store(false, Ordering::Release);
    }
}

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
            .filter(|&v| v > 0)
            .unwrap_or(3600);
        let enabled = cfg.get("gc_enabled").map(|s| s == "true").unwrap_or(false);
        Self {
            interval_secs,
            enabled,
        }
    }
}

/// Iteratively mark all commits, manifests, and data chunks reachable from `start_commit_id`.
///
/// **Fail-closed:** returns `Err` immediately on any missing or corrupt commit/manifest object
/// so GC will NEVER proceed to sweep on an incomplete reachability graph.
pub fn collect_reachable(
    store: &Store,
    start_commit_id: &str,
    seen_commits: &mut HashSet<String>,
    reachable: &mut HashSet<String>,
) -> Result<()> {
    let mut stack = vec![start_commit_id.to_string()];

    while let Some(commit_id) = stack.pop() {
        if !seen_commits.insert(commit_id.clone()) {
            continue;
        }
        reachable.insert(commit_id.clone());

        let commit = load_commit(store, &commit_id)?;
        for manifest_id in &commit.manifests {
            if reachable.insert(manifest_id.clone()) {
                let data = store.get_chunk(manifest_id).map_err(|e| {
                    anyhow::anyhow!(
                        "GC mark failed: missing manifest '{}' referenced by commit '{}': {}",
                        manifest_id,
                        commit_id,
                        e
                    )
                })?;
                let manifest: FileManifest = metadata::deserialize(&data).map_err(|e| {
                    anyhow::anyhow!(
                        "GC mark failed: corrupt manifest '{}' referenced by commit '{}': {}",
                        manifest_id,
                        commit_id,
                        e
                    )
                })?;
                for chunk_id in &manifest.chunks {
                    reachable.insert(chunk_id.clone());
                }
            }
        }
        for parent_id in &commit.parents {
            if !seen_commits.contains(parent_id) {
                stack.push(parent_id.clone());
            }
        }
    }
    Ok(())
}

fn load_commit(store: &Store, commit_id: &str) -> Result<Commit> {
    let data = store.get_chunk(commit_id)?;
    let mut commit: Commit = metadata::deserialize(&data)?;
    commit.commit_id = commit_id.to_string();
    Ok(commit)
}

/// Execute crash-safe, fail-closed garbage collection under an exclusive repository lock.
pub fn gc(path: &Path, json: bool) -> Result<()> {
    let _gc_guard = GcRunningGuard::acquire()?;
    crate::with_write_op(path, "gc", || gc_inner(path, json))
}

fn gc_inner(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_prune
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("not a shard repository (run `shard init` first)");
    }

    let cfg = crate::load_config(&shard_dir)?;
    let fmt = MetadataFormat::from_config(&cfg);
    let store = Store::open(&shard_dir)?;
    let mut reachable: HashSet<String> = HashSet::new();
    let mut seen_commits: HashSet<String> = HashSet::new();

    // 1. Mark from HEAD
    let (_, head_commit) = branch::resolve_head(&shard_dir)?;
    if let Some(ref head) = head_commit {
        collect_reachable(&store, head, &mut seen_commits, &mut reachable)?;
    }

    // 2. Mark from all branches (recursive, including slashed branches like feature/foo)
    let (_, branches) = branch::list_branches(&shard_dir)?;
    for (_, commit_id) in branches {
        if !commit_id.is_empty() {
            collect_reachable(&store, &commit_id, &mut seen_commits, &mut reachable)?;
        }
    }

    // 3. Mark from all tags
    let tags = crate::load_tags(&shard_dir)?;
    for commit_id in tags.values() {
        collect_reachable(&store, commit_id, &mut seen_commits, &mut reachable)?;
    }

    // 4. Mark from staging index
    let index = Index::load(&shard_dir.join("index"), &fmt)?;
    for manifest in index.files.values() {
        let json_bytes = metadata::serialize(manifest, &fmt);
        let hash = blake3::hash(&json_bytes);
        reachable.insert(hash.to_hex().to_string());
        for chunk_hash in &manifest.chunks {
            reachable.insert(chunk_hash.clone());
        }
    }

    // 5. Mark KeyRotation objects stored in the DAG
    let keys_dir = shard_dir.join("keys");
    if keys_dir.exists() {
        let rotations = keychain::load_rotations(&keys_dir)?;
        for rot in &rotations {
            reachable.insert(rot.rotation_id.clone());
            if let Ok(raw) = serde_json::to_vec(rot) {
                reachable.insert(blake3::hash(&raw).to_hex().to_string());
            }
        }
    }

    // 6. Mark in-flight partial transfers
    for obj_id in partial::collect_partial_object_ids(&shard_dir)? {
        reachable.insert(obj_id);
    }

    // Retain objects received from peers until their durable pins expire.
    for object_id in crate::inbound_pins::active_object_ids(&shard_dir)? {
        reachable.insert(object_id);
    }

    // 8. Sweep unreachable objects (only reached if 100% of mark phase succeeded!)
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
    store.flush()?;

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "pruned": pruned,
                "remaining": kept,
            }))?
        );
    } else {
        println!("Pruned {} objects. {} objects remain.", pruned, kept);
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
                let p = path.clone();
                match tokio::task::spawn_blocking(move || gc(&p, false)).await {
                    Ok(Ok(())) => info!("Auto-GC: completed in {:?}", start.elapsed()),
                    Ok(Err(e)) => warn!("Auto-GC: error: {}", e),
                    Err(e) => warn!("Auto-GC task panicked: {}", e),
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
    use crate::chunker::Chunk;
    use crate::commit::Commit;
    use crate::manifest::FileManifest;
    use crate::metadata;
    use tempfile::tempdir;

    static GC_POLICY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn make_chunk(data: &[u8]) -> Chunk {
        let hash = blake3::hash(data);
        Chunk {
            hash,
            data: data.to_vec(),
            offset: 0,
        }
    }

    fn hex_hash(data: &[u8]) -> String {
        blake3::hash(data).to_hex().to_string()
    }

    fn store_commit_chain(
        store: &Store,
        parents: Vec<String>,
        manifests_data: Vec<(&str, &[&[u8]])>,
        timestamp: u64,
    ) -> (String, Vec<String>, Vec<String>) {
        let mut manifest_ids = Vec::new();
        let mut all_chunk_ids = Vec::new();

        for (name, chunks_data) in &manifests_data {
            let mut chunk_ids = Vec::new();
            for d in *chunks_data {
                let c = make_chunk(d);
                let id = c.hash.to_hex().to_string();
                store.put_chunk(&c).unwrap();
                chunk_ids.push(id);
            }
            all_chunk_ids.extend(chunk_ids.clone());

            let manifest = FileManifest {
                name: name.to_string(),
                size: chunks_data.iter().map(|d| d.len() as u64).sum(),
                chunks: chunk_ids,
                content_type: None,
                compression: "none".into(),
                merkle_root: None,
                created_by: None,
                created_at: None,
                signature: None,
            };
            let m_bytes = metadata::serialize(&manifest, &MetadataFormat::Json);
            let m_id = hex_hash(&m_bytes);
            store.put_chunk(&make_chunk(&m_bytes)).unwrap();
            manifest_ids.push(m_id);
        }

        let commit = Commit {
            commit_id: String::new(),
            parents,
            manifests: manifest_ids.clone(),
            author: "gc-test".into(),
            message: "test".into(),
            timestamp,
            public_key: None,
            signature: None,
            key_id: None,
        };
        let c_bytes = metadata::serialize(&commit, &MetadataFormat::Json);
        let c_id = hex_hash(&c_bytes);
        store.put_chunk(&make_chunk(&c_bytes)).unwrap();

        (c_id, manifest_ids, all_chunk_ids)
    }

    fn init_shard_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let shard = dir.path().join(".shard");
        std::fs::create_dir_all(shard.join("objects")).unwrap();

        let mut cfg_map = std::collections::BTreeMap::<String, String>::new();
        cfg_map.insert("storage_backend".into(), "flat".into());
        let cfg_bytes = metadata::serialize(&cfg_map, &MetadataFormat::Json);
        std::fs::write(shard.join("config.json"), cfg_bytes).unwrap();

        (dir, shard)
    }

    #[test]
    fn test_gc_policy_default() {
        let _env_guard = GC_POLICY_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let policy = GcPolicy::from_config(dir.path());
        assert_eq!(policy.interval_secs, 3600);
        assert!(!policy.enabled);
    }

    #[test]
    fn test_gc_policy_from_config_file() {
        let _env_guard = GC_POLICY_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut cfg: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        cfg.insert("gc_interval_secs".into(), "7200".into());
        cfg.insert("gc_enabled".into(), "true".into());
        let bytes = metadata::serialize(&cfg, &MetadataFormat::Json);
        std::fs::write(dir.path().join("config.json"), bytes).unwrap();

        let policy = GcPolicy::from_config(dir.path());
        assert_eq!(policy.interval_secs, 7200);
        assert!(policy.enabled);
    }

    #[test]
    fn test_gc_policy_partial_config() {
        let _env_guard = GC_POLICY_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut cfg: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        cfg.insert("gc_interval_secs".into(), "1800".into());
        let bytes = metadata::serialize(&cfg, &MetadataFormat::Json);
        std::fs::write(dir.path().join("config.json"), bytes).unwrap();

        let policy = GcPolicy::from_config(dir.path());
        assert_eq!(policy.interval_secs, 1800);
        assert!(!policy.enabled);
    }

    #[test]
    fn test_gc_policy_invalid_values_use_defaults() {
        let _env_guard = GC_POLICY_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut cfg: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        cfg.insert("gc_interval_secs".into(), "not-a-number".into());
        cfg.insert("gc_enabled".into(), "maybe".into());
        let bytes = metadata::serialize(&cfg, &MetadataFormat::Json);
        std::fs::write(dir.path().join("config.json"), bytes).unwrap();

        let policy = GcPolicy::from_config(dir.path());
        assert_eq!(policy.interval_secs, 3600);
        assert!(!policy.enabled);
    }

    #[test]
    fn test_gc_policy_env_override_works() {
        let _env_guard = GC_POLICY_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let mut cfg: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        cfg.insert("gc_interval_secs".into(), "3600".into());
        cfg.insert("gc_enabled".into(), "false".into());
        let bytes = metadata::serialize(&cfg, &MetadataFormat::Json);
        std::fs::write(dir.path().join("config.json"), bytes).unwrap();

        let saved_interval = std::env::var("SHARD_GC_INTERVAL_SECS").ok();
        let saved_enabled = std::env::var("SHARD_GC_ENABLED").ok();
        std::env::set_var("SHARD_GC_INTERVAL_SECS", "9999");
        std::env::set_var("SHARD_GC_ENABLED", "true");

        let policy = GcPolicy::from_config(dir.path());

        if let Some(v) = saved_interval {
            std::env::set_var("SHARD_GC_INTERVAL_SECS", v);
        } else {
            std::env::remove_var("SHARD_GC_INTERVAL_SECS");
        }
        if let Some(v) = saved_enabled {
            std::env::set_var("SHARD_GC_ENABLED", v);
        } else {
            std::env::remove_var("SHARD_GC_ENABLED");
        }

        assert_eq!(policy.interval_secs, 9999);
        assert!(policy.enabled);
    }

    #[test]
    fn test_collect_reachable_missing_commit_fails_closed() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("objects")).unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut reachable = HashSet::new();
        let result =
            collect_reachable(&store, &"0".repeat(64), &mut HashSet::new(), &mut reachable);
        assert!(
            result.is_err(),
            "GC mark phase must fail closed on missing commit"
        );
    }

    #[test]
    fn test_collect_reachable_root_commit() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("objects")).unwrap();
        let store = Store::open(dir.path()).unwrap();

        let (commit_id, manifest_ids, chunk_ids) =
            store_commit_chain(&store, vec![], vec![("f.txt", &[b"hello" as &[u8]])], 100);

        let mut reachable = HashSet::new();
        collect_reachable(&store, &commit_id, &mut HashSet::new(), &mut reachable).unwrap();

        assert!(reachable.contains(&commit_id));
        assert!(reachable.contains(&manifest_ids[0]));
        assert!(reachable.contains(&chunk_ids[0]));
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn test_collect_reachable_with_parents() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("objects")).unwrap();
        let store = Store::open(dir.path()).unwrap();

        let (parent_id, _, _) = store_commit_chain(
            &store,
            vec![],
            vec![("parent.txt", &[b"parent" as &[u8]])],
            1,
        );
        let (child_id, _, _) = store_commit_chain(
            &store,
            vec![parent_id.clone()],
            vec![("child.txt", &[b"child" as &[u8]])],
            2,
        );

        let mut reachable = HashSet::new();
        collect_reachable(&store, &child_id, &mut HashSet::new(), &mut reachable).unwrap();

        assert!(reachable.contains(&child_id));
        assert!(reachable.contains(&parent_id));
    }

    #[test]
    fn test_collect_reachable_diamond_merge() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("objects")).unwrap();
        let store = Store::open(dir.path()).unwrap();

        let (root_id, _, _) =
            store_commit_chain(&store, vec![], vec![("root.txt", &[b"root" as &[u8]])], 1);
        let (left_id, _, _) = store_commit_chain(
            &store,
            vec![root_id.clone()],
            vec![("l.txt", &[b"left" as &[u8]])],
            2,
        );
        let (right_id, _, _) = store_commit_chain(
            &store,
            vec![root_id.clone()],
            vec![("r.txt", &[b"right" as &[u8]])],
            3,
        );
        let (merge_id, _, _) = store_commit_chain(
            &store,
            vec![left_id.clone(), right_id.clone()],
            vec![("m.txt", &[b"merge" as &[u8]])],
            4,
        );

        let mut reachable = HashSet::new();
        collect_reachable(&store, &merge_id, &mut HashSet::new(), &mut reachable).unwrap();

        assert!(reachable.contains(&merge_id));
        assert!(reachable.contains(&left_id));
        assert!(reachable.contains(&right_id));
        assert!(reachable.contains(&root_id));
    }

    #[test]
    fn test_gc_preserves_slashed_branch_and_partial_transfers() {
        let (_dir, shard) = init_shard_repo();
        let store = Store::open(&shard).unwrap();

        let (c_main, _, _) =
            store_commit_chain(&store, vec![], vec![("a.txt", &[b"main" as &[u8]])], 1);
        let (c_feat, _, feat_chunks) =
            store_commit_chain(&store, vec![], vec![("b.txt", &[b"feature" as &[u8]])], 2);

        // Partial transfer chunk that is NOT referenced by any commit yet
        let partial_chunk = make_chunk(b"in-flight transfer");
        let partial_chunk_id = partial_chunk.hash.to_hex().to_string();
        store.put_chunk(&partial_chunk).unwrap();
        let partial_commit_id = "1".repeat(64);
        let pt = partial::PartialTransfer::new(&shard, &partial_commit_id).unwrap();
        pt.save_chunk(&partial_chunk_id, b"in-flight transfer")
            .unwrap();

        // Orphan chunk
        let orphan = make_chunk(b"orphan");
        let orphan_id = orphan.hash.to_hex().to_string();
        store.put_chunk(&orphan).unwrap();

        branch::set_head_branch(&shard, "main").unwrap();
        branch::update_branch_ref(&shard, "main", &c_main).unwrap();
        branch::update_branch_ref(&shard, "feature/deep/branch", &c_feat).unwrap();

        gc_inner(_dir.path(), false).unwrap();

        assert!(store.has_chunk(&c_main));
        assert!(store.has_chunk(&c_feat));
        assert!(store.has_chunk(&feat_chunks[0]));
        assert!(store.has_chunk(&partial_chunk_id));
        assert!(!store.has_chunk(&orphan_id));
    }

    #[test]
    fn test_gc_aborts_without_deleting_on_corrupt_ref() {
        let (_dir, shard) = init_shard_repo();
        let store = Store::open(&shard).unwrap();

        let live = make_chunk(b"keep me");
        let live_id = live.hash.to_hex().to_string();
        store.put_chunk(&live).unwrap();

        // Point HEAD to a missing commit
        std::fs::write(shard.join("HEAD"), "f".repeat(64)).unwrap();

        assert!(gc_inner(_dir.path(), false).is_err());
        // Fail closed: NOTHING was deleted!
        assert!(store.has_chunk(&live_id));
    }

    #[test]
    fn test_gc_inner_non_shard_directory() {
        let dir = tempdir().unwrap();
        let result = gc_inner(dir.path(), false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a shard repository"));
    }

    #[test]
    fn test_gc_inner_empty_store() {
        let (_dir, _shard) = init_shard_repo();
        assert!(gc_inner(_dir.path(), false).is_ok());
    }

    #[test]
    fn test_gc_inner_no_head_prunes_everything() {
        let (_dir, shard) = init_shard_repo();
        let store = Store::open(&shard).unwrap();

        let c = make_chunk(b"orphan data");
        let orphan_id = c.hash.to_hex().to_string();
        store.put_chunk(&c).unwrap();

        gc_inner(_dir.path(), false).unwrap();
        assert!(!store.has_chunk(&orphan_id));
    }

    #[test]
    fn test_gc_inner_distinguishes_reachable_and_unreachable() {
        let (_dir, shard) = init_shard_repo();
        let store = Store::open(&shard).unwrap();

        let (commit_id, manifest_ids, chunk_ids) = store_commit_chain(
            &store,
            vec![],
            vec![("keep.txt", &[b"keep data" as &[u8]])],
            1,
        );
        std::fs::write(shard.join("HEAD"), &commit_id).unwrap();

        let unreachable = make_chunk(b"dead data");
        let unreachable_id = unreachable.hash.to_hex().to_string();
        store.put_chunk(&unreachable).unwrap();

        gc_inner(_dir.path(), false).unwrap();

        assert!(store.has_chunk(&commit_id));
        assert!(store.has_chunk(&manifest_ids[0]));
        assert!(store.has_chunk(&chunk_ids[0]));
        assert!(!store.has_chunk(&unreachable_id));
    }

    #[test]
    fn test_gc_inner_with_branch_and_tag() {
        let (_dir, shard) = init_shard_repo();
        let store = Store::open(&shard).unwrap();

        let (c1_id, _, _) =
            store_commit_chain(&store, vec![], vec![("a.txt", &[b"alpha" as &[u8]])], 1);
        let (c2_id, _, _) =
            store_commit_chain(&store, vec![], vec![("b.txt", &[b"beta" as &[u8]])], 2);

        let c3_chunk = make_chunk(b"gamma chunk");
        let c3_id = c3_chunk.hash.to_hex().to_string();
        store.put_chunk(&c3_chunk).unwrap();

        branch::set_head_branch(&shard, "main").unwrap();
        branch::update_branch_ref(&shard, "main", &c1_id).unwrap();

        let mut tags: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        tags.insert("v1".into(), c2_id.clone());
        std::fs::write(
            shard.join("tags.json"),
            serde_json::to_vec_pretty(&tags).unwrap(),
        )
        .unwrap();

        assert!(gc_inner(_dir.path(), false).is_ok());

        assert!(store.has_chunk(&c1_id));
        assert!(store.has_chunk(&c2_id));
        assert!(!store.has_chunk(&c3_id));
    }

    #[test]
    fn test_gc_concurrency_guard() {
        let (_dir, shard) = init_shard_repo();
        let store = Store::open(&shard).unwrap();

        let (c_id, _, _) = store_commit_chain(&store, vec![], vec![("x.txt", &[b"x" as &[u8]])], 1);
        std::fs::write(shard.join("HEAD"), &c_id).unwrap();

        assert!(gc(_dir.path(), false).is_ok());
        assert!(gc(_dir.path(), false).is_ok());
    }

    #[test]
    fn test_gc_inner_json_output() {
        let (_dir, _shard) = init_shard_repo();
        assert!(gc_inner(_dir.path(), true).is_ok());
    }
}
