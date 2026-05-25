use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

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
    /// Iterate over all key-value pairs whose keys start with `prefix`.
    fn iter_prefix(&self, prefix: &[u8]) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

/// Sled-based storage backend. Persists to a sled database directory.
pub struct SledBackend {
    db: sled::Db,
}

impl SledBackend {
    /// Open or create a sled database at `path`.
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let db = sled::open(path)?;
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
}

/// SQLite-based storage backend. Persists to a single `.db` file.
pub struct SqliteBackend {
    conn: Mutex<Connection>,
}

impl SqliteBackend {
    /// Open or create a SQLite database at `path`.
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS shard_store (
                key   TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl StorageBackend for SqliteBackend {
    fn put(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO shard_store (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }

    fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM shard_store WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    fn delete(&self, key: &[u8]) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM shard_store WHERE key = ?1", [key])?;
        Ok(())
    }

    fn contains(&self, key: &[u8]) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT 1 FROM shard_store WHERE key = ?1")?;
        let exists = stmt.exists([key])?;
        Ok(exists)
    }

    fn flush(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    fn iter_prefix(&self, prefix: &[u8]) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("{}%", std::str::from_utf8(prefix).unwrap_or(""));
        let mut stmt =
            conn.prepare("SELECT key, value FROM shard_store WHERE key LIKE ?1 ORDER BY key")?;
        let rows = stmt.query_map([&pattern], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
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
    fn test_sqlite_overwrite() {
        let dir = tempdir().unwrap();
        let backend: Box<dyn StorageBackend> =
            Box::new(SqliteBackend::new(&dir.path().join("o")).unwrap());
        backend.put(b"k", b"v1").unwrap();
        backend.put(b"k", b"v2").unwrap();
        assert_eq!(backend.get(b"k").unwrap(), Some(b"v2".to_vec()));
    }
}
