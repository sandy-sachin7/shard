use crate::chunker::Chunk;
use anyhow::Result;
use shard_storage::{open_backend, StorageBackend};
use std::path::Path;

pub struct SqliteStore {
    backend: Box<dyn StorageBackend>,
}

impl SqliteStore {
    pub fn new(root: &Path) -> Result<Self> {
        let db_path = root.join("objects.db");
        let backend = open_backend(&db_path, "sqlite")?;
        Ok(Self { backend })
    }

    pub fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        let hash_hex = chunk.hash.to_hex().to_string();
        if !self.backend.contains(hash_hex.as_bytes())? {
            self.backend.put(hash_hex.as_bytes(), &chunk.data)?;
        }
        Ok(())
    }

    pub fn get_chunk(&self, hash_hex: &str) -> Result<Vec<u8>> {
        self.backend
            .get(hash_hex.as_bytes())?
            .ok_or_else(|| anyhow::anyhow!("Chunk not found: {}", hash_hex))
    }

    pub fn has_chunk(&self, hash_hex: &str) -> bool {
        self.backend.contains(hash_hex.as_bytes()).unwrap_or(false)
    }

    pub fn iter_chunks(&self) -> Result<Vec<(String, String)>> {
        let results = self.backend.iter_prefix(b"")?;
        Ok(results
            .into_iter()
            .map(|(k, _)| {
                let hash = String::from_utf8_lossy(&k).to_string();
                (hash.clone(), hash)
            })
            .collect())
    }

    pub fn delete_chunk(&self, hash_hex: &str, _full_path: Option<&str>) -> Result<()> {
        self.backend.delete(hash_hex.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::Chunk;
    use tempfile::tempdir;

    fn fake_chunk(data: &[u8]) -> Chunk {
        Chunk {
            hash: blake3::hash(data),
            data: data.to_vec(),
            offset: 0,
        }
    }

    #[test]
    fn test_sqlite_put_get_roundtrip() {
        let dir = tempdir().unwrap();
        let store = SqliteStore::new(dir.path()).unwrap();
        let chunk = fake_chunk(b"sqlite test data");
        store.put_chunk(&chunk).unwrap();
        let hash_hex = chunk.hash.to_hex().to_string();
        assert!(store.has_chunk(&hash_hex));
        let retrieved = store.get_chunk(&hash_hex).unwrap();
        assert_eq!(retrieved, b"sqlite test data");
    }

    #[test]
    fn test_sqlite_get_nonexistent() {
        let dir = tempdir().unwrap();
        let store = SqliteStore::new(dir.path()).unwrap();
        let result = store.get_chunk("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_sqlite_delete_chunk() {
        let dir = tempdir().unwrap();
        let store = SqliteStore::new(dir.path()).unwrap();
        let chunk = fake_chunk(b"sqlite delete");
        store.put_chunk(&chunk).unwrap();
        let hash_hex = chunk.hash.to_hex().to_string();
        assert!(store.has_chunk(&hash_hex));
        store.delete_chunk(&hash_hex, None).unwrap();
        assert!(!store.has_chunk(&hash_hex));
    }

    #[test]
    fn test_sqlite_iter_chunks() {
        let dir = tempdir().unwrap();
        let store = SqliteStore::new(dir.path()).unwrap();
        let chunks = vec![
            fake_chunk(b"sqlite a"),
            fake_chunk(b"sqlite b"),
        ];
        for c in &chunks {
            store.put_chunk(c).unwrap();
        }
        let entries = store.iter_chunks().unwrap();
        assert_eq!(entries.len(), 2);
    }
}
