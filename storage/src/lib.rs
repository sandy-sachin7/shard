//! Pluggable persistent key-value storage backends for the Shard distributed VCS.
//!
//! This crate provides the [`StorageBackend`] trait along with two embedded
//! database implementations:
//!
//! - [`SledBackend`]: Lock-free embedded B-tree database with in-process handle
//!   deduplication so nested opens on the same repository path never conflict.
//! - [`SqliteBackend`]: Pooled SQLite database (`r2d2`) configured with WAL
//!   journal mode, `synchronous=NORMAL`, and `busy_timeout=5000`.
//!
//! # Quickstart
//!
//! ```rust
//! use shard_storage::{open_backend, StorageBackend};
//!
//! # fn main() -> anyhow::Result<()> {
//! let dir = tempfile::tempdir()?;
//! let store = open_backend(&dir.path().join("objects.db"), "sqlite")?;
//! store.put(b"chunk-hash", b"payload")?;
//! assert_eq!(store.get(b"chunk-hash")?.as_deref(), Some(b"payload".as_slice()));
//! store.checkpoint_for_snapshot()?;
//! # Ok(())
//! # }
//! ```

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Trait for content-addressed storage backends used by Shard.
pub trait StorageBackend: Send + Sync {
    /// Store a value under the given key.
    fn put(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()>;
    /// Retrieve the value stored under the given key, or `None` if absent.
    fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>>;
    /// Delete the value stored under the given key.
    fn delete(&self, key: &[u8]) -> anyhow::Result<()>;
    /// Returns `true` if the given key exists in the store.
    fn contains(&self, key: &[u8]) -> anyhow::Result<bool>;
    /// Flush any buffered writes to persistent storage.
    fn flush(&self) -> anyhow::Result<()>;
    /// Flush and fully checkpoint on-disk state before creating a standalone file snapshot
    /// (e.g., `PRAGMA wal_checkpoint(TRUNCATE)` on SQLite so `objects.db` is self-contained).
    fn checkpoint_for_snapshot(&self) -> anyhow::Result<()> {
        self.flush()
    }
    /// Iterate over all key-value pairs whose keys start with `prefix`.
    fn iter_prefix(&self, prefix: &[u8]) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>>;
    /// Iterate over all keys starting with `prefix` without loading blob values into memory.
    fn iter_keys(&self, prefix: &[u8]) -> anyhow::Result<Vec<Vec<u8>>>;
}

static SLED_HANDLE_CACHE: OnceLock<Mutex<HashMap<PathBuf, Weak<sled::Db>>>> = OnceLock::new();

fn sled_handle_cache() -> &'static Mutex<HashMap<PathBuf, Weak<sled::Db>>> {
    SLED_HANDLE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn canonical_sled_key(path: &Path) -> PathBuf {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return canon;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        let parent_dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let Ok(canon_parent) = std::fs::canonicalize(parent_dir) {
            return canon_parent.join(name);
        }
    }
    path.to_path_buf()
}

/// Sled-based storage backend. Persists to a sled database directory.
/// Shares a single live `Arc<sled::Db>` per canonical path within the process so
/// concurrent/nested opens (e.g., `sync()` auto-pull or background GC) never fail
/// with Sled's exclusive file-lock error.
pub struct SledBackend {
    db: Arc<sled::Db>,
}

impl SledBackend {
    /// Open or create a sled database at `path`, reusing any live in-process handle
    /// for the same canonical path.
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let mut cache = sled_handle_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, weak| weak.strong_count() > 0);

        let key = canonical_sled_key(path);
        if let Some(existing) = cache.get(&key).and_then(Weak::upgrade) {
            return Ok(Self { db: existing });
        }

        let db = Arc::new(sled::open(path)?);
        let final_key = canonical_sled_key(path);
        cache.insert(final_key, Arc::downgrade(&db));
        Ok(Self { db })
    }
}

impl StorageBackend for SledBackend {
    fn put(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        self.db.insert(key, value)?;
        Ok(())
    }

    fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.db.get(key)?.map(|v| v.to_vec()))
    }

    fn delete(&self, key: &[u8]) -> anyhow::Result<()> {
        self.db.remove(key)?;
        Ok(())
    }

    fn contains(&self, key: &[u8]) -> anyhow::Result<bool> {
        Ok(self.db.contains_key(key)?)
    }

    fn flush(&self) -> anyhow::Result<()> {
        self.db.flush()?;
        Ok(())
    }

    fn iter_prefix(&self, prefix: &[u8]) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut results = Vec::new();
        for result in self.db.scan_prefix(prefix) {
            let (k, v) = result?;
            results.push((k.to_vec(), v.to_vec()));
        }
        Ok(results)
    }

    fn iter_keys(&self, prefix: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut results = Vec::new();
        for result in self.db.scan_prefix(prefix).keys() {
            let k = result?;
            results.push(k.to_vec());
        }
        Ok(results)
    }
}

/// SQLite-based storage backend with connection pooling. Persists to a single `.db` file.
pub struct SqliteBackend {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteBackend {
    /// Open or create a SQLite database at `path` with a connection pool.
    /// Configures WAL journal mode, `synchronous=NORMAL`, and `busy_timeout=5000`
    /// on every connection in the pool.
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;",
            )
        });
        let pool = Pool::builder().max_size(8).build(manager)?;
        {
            let conn = pool.get()?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS shard_store (
                    key   BLOB PRIMARY KEY,
                    value BLOB NOT NULL
                );",
            )?;
        }
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool<SqliteConnectionManager> {
        &self.pool
    }
}

impl StorageBackend for SqliteBackend {
    fn put(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR REPLACE INTO shard_store (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare_cached("SELECT value FROM shard_store WHERE key = ?1")?;
        let mut rows = stmt.query(rusqlite::params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    fn delete(&self, key: &[u8]) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM shard_store WHERE key = ?1",
            rusqlite::params![key],
        )?;
        Ok(())
    }

    fn contains(&self, key: &[u8]) -> anyhow::Result<bool> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare_cached("SELECT 1 FROM shard_store WHERE key = ?1")?;
        let exists = stmt.exists(rusqlite::params![key])?;
        Ok(exists)
    }

    fn flush(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
        Ok(())
    }

    fn checkpoint_for_snapshot(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        let (busy, log_frames, checkpointed_frames): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 || log_frames != checkpointed_frames {
            anyhow::bail!(
                "SQLite WAL truncate checkpoint incomplete (busy={}, log_frames={}, checkpointed_frames={})",
                busy,
                log_frames,
                checkpointed_frames
            );
        }
        Ok(())
    }

    fn iter_prefix(&self, prefix: &[u8]) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let conn = self.pool.get()?;
        let mut results = Vec::new();
        if prefix.is_empty() {
            let mut stmt =
                conn.prepare_cached("SELECT key, value FROM shard_store ORDER BY key")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let mut stmt = conn.prepare_cached(
                "SELECT key, value FROM shard_store WHERE substr(key, 1, ?1) = ?2 ORDER BY key",
            )?;
            let rows = stmt.query_map(rusqlite::params![prefix.len() as i64, prefix], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                results.push(row?);
            }
        }
        Ok(results)
    }

    fn iter_keys(&self, prefix: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        let conn = self.pool.get()?;
        let mut results = Vec::new();
        if prefix.is_empty() {
            let mut stmt = conn.prepare_cached("SELECT key FROM shard_store ORDER BY key")?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let mut stmt = conn.prepare_cached(
                "SELECT key FROM shard_store WHERE substr(key, 1, ?1) = ?2 ORDER BY key",
            )?;
            let rows = stmt.query_map(rusqlite::params![prefix.len() as i64, prefix], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            for row in rows {
                results.push(row?);
            }
        }
        Ok(results)
    }
}

/// Open a storage backend by name. Currently supports `"sled"` or `"sqlite"`.
pub fn open_backend(path: &Path, backend: &str) -> anyhow::Result<Box<dyn StorageBackend>> {
    match backend {
        "sled" => Ok(Box::new(SledBackend::new(path)?)),
        "sqlite" => Ok(Box::new(SqliteBackend::new(path)?)),
        _ => anyhow::bail!("Unknown storage backend: {}", backend),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_backend_roundtrip(backend: &dyn StorageBackend) {
        backend.put(b"key1", b"value1").unwrap();
        backend.put(b"key2", b"value2").unwrap();

        assert!(backend.contains(b"key1").unwrap());
        assert!(!backend.contains(b"nonexistent").unwrap());

        assert_eq!(backend.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(backend.get(b"key2").unwrap(), Some(b"value2".to_vec()));
        assert!(backend.get(b"nonexistent").unwrap().is_none());

        // iter_prefix on all keys
        let all = backend.iter_prefix(b"").unwrap();
        assert_eq!(all.len(), 2);

        // iter_keys on all keys and prefix
        let all_keys = backend.iter_keys(b"").unwrap();
        assert_eq!(all_keys.len(), 2);
        assert_eq!(backend.iter_keys(b"key").unwrap().len(), 2);
        assert_eq!(backend.iter_keys(b"nope").unwrap().len(), 0);

        // iter_prefix on prefix
        let k1 = backend.iter_prefix(b"key").unwrap();
        assert_eq!(k1.len(), 2);

        let kn = backend.iter_prefix(b"nope").unwrap();
        assert_eq!(kn.len(), 0);

        // delete
        backend.delete(b"key1").unwrap();
        assert!(!backend.contains(b"key1").unwrap());

        backend.flush().unwrap();
    }

    #[test]
    fn test_sled_backend_roundtrip() {
        let dir = tempdir().unwrap();
        let backend: Box<dyn StorageBackend> =
            Box::new(SledBackend::new(&dir.path().join("sled_db")).unwrap());
        test_backend_roundtrip(backend.as_ref());
    }

    #[test]
    fn test_sqlite_backend_roundtrip() {
        let dir = tempdir().unwrap();
        let backend: Box<dyn StorageBackend> =
            Box::new(SqliteBackend::new(&dir.path().join("sqlite.db")).unwrap());
        test_backend_roundtrip(backend.as_ref());
    }

    #[test]
    fn test_sqlite_wal_and_pragmas_enabled() {
        let dir = tempdir().unwrap();
        let backend = SqliteBackend::new(&dir.path().join("sqlite.db")).unwrap();
        let conn = backend.pool().get().unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);
    }

    #[test]
    fn test_open_backend_factory() {
        let dir = tempdir().unwrap();
        let sled = open_backend(&dir.path().join("s.db"), "sled").unwrap();
        let sqlite = open_backend(&dir.path().join("q.db"), "sqlite").unwrap();
        sled.put(b"a", b"1").unwrap();
        sqlite.put(b"a", b"1").unwrap();
        assert_eq!(sled.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(sqlite.get(b"a").unwrap(), Some(b"1".to_vec()));
    }

    #[test]
    fn test_open_backend_unknown() {
        let dir = tempdir().unwrap();
        let result = open_backend(&dir.path().join("x"), "unknown");
        assert!(result.is_err());
    }

    #[test]
    fn test_sled_overwrite() {
        let dir = tempdir().unwrap();
        let backend: Box<dyn StorageBackend> =
            Box::new(SledBackend::new(&dir.path().join("o")).unwrap());
        backend.put(b"k", b"v1").unwrap();
        backend.put(b"k", b"v2").unwrap();
        assert_eq!(backend.get(b"k").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_sled_concurrent_and_nested_reopen_reuses_handle() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("nested_sled");
        let b1 = SledBackend::new(&db_path).unwrap();
        // Non-canonical equivalent path (dir/./nested_sled) must still reuse handle
        let alias_path = dir.path().join(".").join("nested_sled");
        let b2 = SledBackend::new(&alias_path).unwrap();

        b1.put(b"from_b1", b"hello").unwrap();
        assert_eq!(b2.get(b"from_b1").unwrap(), Some(b"hello".to_vec()));

        // Multi-threaded concurrent opens + writes on the same path must never fail
        let mut threads = Vec::new();
        for i in 0..6 {
            let p = db_path.clone();
            threads.push(std::thread::spawn(move || {
                let handle = SledBackend::new(&p).unwrap();
                let key = format!("thread_{}", i);
                handle.put(key.as_bytes(), b"ok").unwrap();
                assert_eq!(handle.get(key.as_bytes()).unwrap(), Some(b"ok".to_vec()));
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(b1.iter_keys(b"thread_").unwrap().len(), 6);
    }

    #[test]
    fn test_sqlite_overwrite() {
        let dir = tempdir().unwrap();
        let backend: Box<dyn StorageBackend> =
            Box::new(SqliteBackend::new(&dir.path().join("o")).unwrap());
        backend.put(b"k", b"v1").unwrap();
        backend.put(b"k", b"v2").unwrap();
        assert_eq!(backend.get(b"k").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_sqlite_checkpoint_for_snapshot_makes_db_standalone() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let backend = SqliteBackend::new(&db_path).unwrap();
        for i in 0..32 {
            backend
                .put(
                    format!("key_{}", i).as_bytes(),
                    format!("val_{}", i).as_bytes(),
                )
                .unwrap();
        }
        backend.checkpoint_for_snapshot().unwrap();

        // Copy ONLY the main .db file (no -wal / -shm) to another directory and verify all rows
        let copy_dir = tempdir().unwrap();
        let copied_db = copy_dir.path().join("objects.db");
        std::fs::copy(&db_path, &copied_db).unwrap();
        let reopened = SqliteBackend::new(&copied_db).unwrap();
        for i in 0..32 {
            assert_eq!(
                reopened.get(format!("key_{}", i).as_bytes()).unwrap(),
                Some(format!("val_{}", i).into_bytes())
            );
        }
    }
}
