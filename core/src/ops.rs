use anyhow::Result;
use fs2::FileExt;
use parking_lot::{ArcRwLockReadGuard, ArcRwLockWriteGuard, RawRwLock, RwLock};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, Debug)]
pub enum OpKind {
    Read,
    Write,
}

impl std::fmt::Display for OpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpKind::Read => write!(f, "read"),
            OpKind::Write => write!(f, "write"),
        }
    }
}

#[derive(Clone)]
struct OpEntry {
    id: String,
    kind: OpKind,
    started: Instant,
    description: String,
}

/// Per-repository concurrency controller combining an in-process `RwLock` with
/// OS-level cross-process advisory file locking on `.shard/repo.lock`.
#[derive(Clone)]
pub struct OpQueue {
    rwlock: Arc<RwLock<()>>,
    active: Arc<RwLock<Vec<OpEntry>>>,
    repo_path: Option<PathBuf>,
}

impl Default for OpQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OpQueue {
    pub fn new() -> Self {
        Self {
            rwlock: Arc::new(RwLock::new(())),
            active: Arc::new(RwLock::new(Vec::new())),
            repo_path: None,
        }
    }

    pub fn for_repo(repo_path: PathBuf) -> Self {
        Self {
            rwlock: Arc::new(RwLock::new(())),
            active: Arc::new(RwLock::new(Vec::new())),
            repo_path: Some(repo_path),
        }
    }

    /// Acquire a real read or write lock immediately (blocking until granted).
    /// Acquires the in-process `RwLock` first, then the OS-level advisory file lock
    /// on `.shard/repo.lock` (if `.shard` exists).
    pub fn acquire(&self, kind: OpKind, description: String) -> OpGuard {
        let mem_lock = match kind {
            OpKind::Read => MemLockGuard::Read(self.rwlock.read_arc()),
            OpKind::Write => MemLockGuard::Write(self.rwlock.write_arc()),
        };

        let file_lock = self
            .repo_path
            .as_deref()
            .and_then(|root| acquire_os_lock(root, kind).ok().flatten());

        let id = uuid::Uuid::new_v4().to_string();
        self.active.write().push(OpEntry {
            id: id.clone(),
            kind,
            started: Instant::now(),
            description,
        });

        OpGuard {
            active: self.active.clone(),
            _file_lock: file_lock,
            _mem_lock: mem_lock,
            id,
            kind,
        }
    }

    /// Snapshot of currently executing operations on this repository.
    pub fn snapshot(&self) -> OpsSnapshot {
        let active = self.active.read();
        OpsSnapshot {
            running: active
                .iter()
                .map(|e| OpSnapshotEntry {
                    id: e.id.clone(),
                    kind: e.kind,
                    elapsed_ms: e.started.elapsed().as_millis() as u64,
                    description: e.description.clone(),
                })
                .collect(),
            pending: 0,
        }
    }
}

fn acquire_os_lock(repo_root: &Path, kind: OpKind) -> Result<Option<File>> {
    let shard_dir = repo_root.join(".shard");
    if !shard_dir.exists() {
        return Ok(None);
    }
    let lock_path = shard_dir.join("repo.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    match kind {
        OpKind::Read => file.lock_shared()?,
        OpKind::Write => file.lock_exclusive()?,
    }
    Ok(Some(file))
}

#[allow(dead_code)]
enum MemLockGuard {
    Read(ArcRwLockReadGuard<RawRwLock, ()>),
    Write(ArcRwLockWriteGuard<RawRwLock, ()>),
}

/// RAII lock guard holding both the OS-level `.shard/repo.lock` advisory lock
/// and the in-process `RwLock` guard. Releases cleanly on drop.
pub struct OpGuard {
    active: Arc<RwLock<Vec<OpEntry>>>,
    // Drop file lock BEFORE in-process RwLock so another thread in this process
    // wakes only after the OS lock is released.
    _file_lock: Option<File>,
    _mem_lock: MemLockGuard,
    pub id: String,
    pub kind: OpKind,
}

impl Drop for OpGuard {
    fn drop(&mut self) {
        self.active.write().retain(|e| e.id != self.id);
        if let Some(ref f) = self._file_lock {
            let _ = f.unlock();
        }
    }
}

#[derive(serde::Serialize)]
pub struct OpsSnapshot {
    pub running: Vec<OpSnapshotEntry>,
    pub pending: u64,
}

#[derive(serde::Serialize)]
pub struct OpSnapshotEntry {
    pub id: String,
    pub kind: OpKind,
    pub elapsed_ms: u64,
    pub description: String,
}

#[derive(Default)]
pub struct RepoOpQueues {
    pub repos: RwLock<HashMap<PathBuf, OpQueue>>,
}

impl RepoOpQueues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create(&self, repo: &Path) -> OpQueue {
        let key = fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
        {
            let map = self.repos.read();
            if let Some(q) = map.get(&key) {
                return q.clone();
            }
        }
        let mut map = self.repos.write();
        map.entry(key.clone())
            .or_insert_with(|| OpQueue::for_repo(key))
            .clone()
    }
}

thread_local! {
    pub static CURRENT_TRACE_ID: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub fn generate_trace_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn set_trace_id(trace_id: &str) {
    CURRENT_TRACE_ID.with(|cell| {
        *cell.borrow_mut() = trace_id.to_string();
    });
}

pub fn get_trace_id() -> String {
    CURRENT_TRACE_ID.with(|cell| cell.borrow().clone())
}

pub fn traced_info(msg: impl std::fmt::Display) {
    let tid = get_trace_id();
    if tid.is_empty() {
        info!("{}", msg);
    } else {
        info!("[{}] {}", tid, msg);
    }
}

pub fn traced_warn(msg: impl std::fmt::Display) {
    let tid = get_trace_id();
    if tid.is_empty() {
        warn!("{}", msg);
    } else {
        warn!("[{}] {}", tid, msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_write_lock_provides_mutual_exclusion() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".shard")).unwrap();
        let queues = Arc::new(RepoOpQueues::new());
        let active_writers = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for i in 0..4 {
            let q = queues.clone();
            let path = dir.path().to_path_buf();
            let aw = active_writers.clone();
            let mc = max_concurrent.clone();
            handles.push(std::thread::spawn(move || {
                let _guard = q
                    .get_or_create(&path)
                    .acquire(OpKind::Write, format!("w{}", i));
                let now = aw.fetch_add(1, Ordering::SeqCst) + 1;
                mc.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(15));
                aw.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(max_concurrent.load(Ordering::SeqCst), 1);
        assert!(queues
            .get_or_create(dir.path())
            .snapshot()
            .running
            .is_empty());
    }

    #[test]
    fn test_read_locks_run_concurrently() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".shard")).unwrap();
        let queue = OpQueue::for_repo(dir.path().to_path_buf());
        let g1 = queue.acquire(OpKind::Read, "r1".into());
        let g2 = queue.acquire(OpKind::Read, "r2".into());
        assert_eq!(queue.snapshot().running.len(), 2);
        drop(g1);
        drop(g2);
        assert_eq!(queue.snapshot().running.len(), 0);
    }
}
