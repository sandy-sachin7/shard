use std::path::Path;

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

/// Open a storage backend by name. Currently supports `"sled"`.
pub fn open_backend(path: &Path, backend: &str) -> anyhow::Result<Box<dyn StorageBackend>> {
    match backend {
        "sled" => Ok(Box::new(SledBackend::new(path)?)),
        _ => anyhow::bail!("Unknown storage backend: {}", backend),
    }
}
