use anyhow::Result;
use fs2::FileExt;
use parking_lot::{Condvar, Mutex, RwLock};
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
/// OS-level cross-process advisory file locking on `.shard.lock`.
#[derive(Clone)]
pub struct OpQueue {
    rwlock: Arc<RepoRwLock>,
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
            rwlock: Arc::new(RepoRwLock::default()),
            active: Arc::new(RwLock::new(Vec::new())),
            repo_path: None,
        }
    }

    pub fn for_repo(repo_path: PathBuf) -> Self {
        Self {
            rwlock: Arc::new(RepoRwLock::default()),
            active: Arc::new(RwLock::new(Vec::new())),
            repo_path: Some(repo_path),
        }
    }

    /// Acquire a read or write lock immediately (blocking until granted).
    /// Acquires the in-process read-write lock first, then the OS-level file lock
    /// on `.shard.lock`.
    pub fn acquire(&self, kind: OpKind, description: String) -> Result<OpGuard> {
        let mem_lock = self.rwlock.acquire(kind);

        let file_lock = match self.repo_path.as_deref() {
            Some(root) => acquire_os_lock(root, kind)?,
            None => None,
        };

        let id = uuid::Uuid::new_v4().to_string();
        self.active.write().push(OpEntry {
            id: id.clone(),
            kind,
            started: Instant::now(),
            description,
        });

        Ok(OpGuard {
            active: self.active.clone(),
            _file_lock: file_lock,
            _mem_lock: mem_lock,
            id,
            kind,
        })
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
    if !repo_root.exists() {
        // Only a write op (typically `init`) may create a repository directory.
        // Read-only commands on a mistyped path must fail without side effects.
        if kind != OpKind::Write {
            return Ok(None);
        }
        fs::create_dir_all(repo_root)?;
    }
    let canonical_root = fs::canonicalize(repo_root)?;
    let lock_path = canonical_root.join(".shard.lock");
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

#[derive(Default)]
struct RepoLockState {
    readers: usize,
    waiting_writers: usize,
    writer: bool,
}

#[derive(Default)]
struct RepoRwLock {
    state: Mutex<RepoLockState>,
    changed: Condvar,
}

impl RepoRwLock {
    fn acquire(self: &Arc<Self>, kind: OpKind) -> MemLockGuard {
        let mut state = self.state.lock();
        match kind {
            OpKind::Read => {
                while state.writer || state.waiting_writers > 0 {
                    self.changed.wait(&mut state);
                }
                state.readers += 1;
            }
            OpKind::Write => {
                state.waiting_writers += 1;
                while state.writer || state.readers > 0 {
                    self.changed.wait(&mut state);
                }
                state.waiting_writers -= 1;
                state.writer = true;
            }
        }
        MemLockGuard {
            lock: self.clone(),
            kind,
        }
    }
}

struct MemLockGuard {
    lock: Arc<RepoRwLock>,
    kind: OpKind,
}

impl Drop for MemLockGuard {
    fn drop(&mut self) {
        let mut state = self.lock.state.lock();
        match self.kind {
            OpKind::Read => state.readers -= 1,
            OpKind::Write => state.writer = false,
        }
        self.lock.changed.notify_all();
    }
}

/// RAII lock guard holding both the OS-level `.shard.lock` advisory lock
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
                    .acquire(OpKind::Write, format!("w{}", i))
                    .unwrap();
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
        let g1 = queue.acquire(OpKind::Read, "r1".into()).unwrap();
        let g2 = queue.acquire(OpKind::Read, "r2".into()).unwrap();
        assert_eq!(queue.snapshot().running.len(), 2);
        drop(g1);
        drop(g2);
        assert_eq!(queue.snapshot().running.len(), 0);
    }

    #[test]
    fn test_os_lock_failure_is_returned() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join(".shard.lock");
        fs::create_dir_all(&lock_path).unwrap();
        let queue = OpQueue::for_repo(dir.path().to_path_buf());

        assert!(queue
            .acquire(OpKind::Write, "write with invalid lock path".into())
            .is_err());
        assert!(queue.snapshot().running.is_empty());
    }

    #[test]
    fn test_lock_exists_before_repo_initialization() {
        let dir = tempdir().unwrap();
        let repo_root = dir.path().join("new-repo");
        let queue = OpQueue::for_repo(repo_root.clone());
        let _guard = queue.acquire(OpKind::Write, "init".into()).unwrap();

        assert!(repo_root.join(".shard.lock").is_file());
        assert!(!repo_root.join(".shard").exists());
    }
}
