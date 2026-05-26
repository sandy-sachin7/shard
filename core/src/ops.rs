use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum OpKind {
    Read,
    Write,
}

#[derive(Clone)]
pub struct OpQueue {
    inner: Arc<RwLock<OpQueueInner>>,
}

struct OpQueueInner {
    running: Vec<OpEntry>,
    pending: Vec<OpEntry>,
}

#[derive(Clone)]
struct OpEntry {
    id: String,
    kind: OpKind,
    started: Instant,
    description: String,
}

impl Default for OpQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OpQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(OpQueueInner {
                running: Vec::new(),
                pending: Vec::new(),
            })),
        }
    }

    pub fn acquire(&self, kind: OpKind, description: String) -> OpGuard {
        let id = uuid::Uuid::new_v4().to_string();
        {
            let mut inner = self.inner.write();
            let has_write = inner.running.iter().any(|e| e.kind == OpKind::Write);
            let has_read_write = if kind == OpKind::Write {
                !inner.running.is_empty()
            } else {
                false
            };
            if has_write || has_read_write {
                inner.pending.push(OpEntry {
                    id: id.clone(),
                    kind,
                    started: Instant::now(),
                    description: description.clone(),
                });
            } else {
                inner.running.push(OpEntry {
                    id: id.clone(),
                    kind,
                    started: Instant::now(),
                    description: description.clone(),
                });
            }
        }
        OpGuard {
            queue: self.inner.clone(),
            id,
            kind,
        }
    }

    pub fn wait_for_turn(&self, guard: &OpGuard) {
        loop {
            let inner = self.inner.read();
            let is_in_running = inner.running.iter().any(|e| e.id == guard.id);
            if is_in_running {
                return;
            }
            let pos = inner.pending.iter().position(|e| e.id == guard.id);
            if let Some(idx) = pos {
                let front_is_write = inner
                    .pending
                    .first()
                    .map(|e| e.kind == OpKind::Write)
                    .unwrap_or(false);
                let is_read = guard.kind == OpKind::Read;
                if idx == 0 && (!front_is_write || !is_read) {
                    drop(inner);
                    let mut inner = self.inner.write();
                    if let Some(entry) = inner
                        .pending
                        .iter()
                        .position(|e| e.id == guard.id)
                        .map(|i| inner.pending.remove(i))
                    {
                        inner.running.push(entry);
                    }
                    return;
                }
            }
            drop(inner);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn snapshot(&self) -> OpsSnapshot {
        let inner = self.inner.read();
        OpsSnapshot {
            running: inner
                .running
                .iter()
                .map(|e| OpSnapshotEntry {
                    id: e.id.clone(),
                    kind: e.kind,
                    elapsed_ms: e.started.elapsed().as_millis() as u64,
                    description: e.description.clone(),
                })
                .collect(),
            pending: inner.pending.len() as u64,
        }
    }
}

pub struct OpGuard {
    queue: Arc<RwLock<OpQueueInner>>,
    pub id: String,
    pub kind: OpKind,
}

impl Drop for OpGuard {
    fn drop(&mut self) {
        let mut inner = self.queue.write();
        inner.running.retain(|e| e.id != self.id);
        promote_pending(&mut inner);
    }
}

fn promote_pending(inner: &mut OpQueueInner) {
    while let Some(entry) = inner.pending.first() {
        let has_write = inner.running.iter().any(|e| e.kind == OpKind::Write);
        let can_advance = match entry.kind {
            OpKind::Read => !has_write,
            OpKind::Write => inner.running.is_empty(),
        };
        if can_advance {
            let e = inner.pending.remove(0);
            inner.running.push(e);
        } else {
            break;
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

impl std::fmt::Display for OpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpKind::Read => write!(f, "read"),
            OpKind::Write => write!(f, "write"),
        }
    }
}

#[derive(Default)]
pub struct RepoOpQueues {
    pub repos: RwLock<HashMap<PathBuf, OpQueue>>,
}

impl RepoOpQueues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create(&self, repo: &PathBuf) -> OpQueue {
        let map = self.repos.read();
        if let Some(q) = map.get(repo) {
            return q.clone();
        }
        drop(map);
        let mut map = self.repos.write();
        map.entry(repo.clone()).or_default().clone()
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
