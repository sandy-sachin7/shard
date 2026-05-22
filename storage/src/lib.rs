use std::path::Path;

pub trait StorageBackend: Send + Sync {
    fn put(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()>;
    fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>>;
    fn delete(&self, key: &[u8]) -> anyhow::Result<()>;
    fn contains(&self, key: &[u8]) -> anyhow::Result<bool>;
    fn flush(&self) -> anyhow::Result<()>;
    fn iter_prefix(&self, prefix: &[u8]) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct SledBackend {
    db: sled::Db,
}

impl SledBackend {
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

pub fn open_backend(path: &Path, backend: &str) -> anyhow::Result<Box<dyn StorageBackend>> {
    match backend {
        "sled" => Ok(Box::new(SledBackend::new(path)?)),
        _ => anyhow::bail!("Unknown storage backend: {}", backend),
    }
}
