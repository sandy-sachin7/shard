use crate::chunker::Chunk;
use crate::fs_util::validate_object_id;
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
        validate_object_id(&hash_hex)?;
        if !self.backend.contains(hash_hex.as_bytes())? {
            self.backend.put(hash_hex.as_bytes(), &chunk.data)?;
        }
        Ok(())
    }

    pub fn put_chunk_overwrite(&self, chunk: &Chunk) -> Result<()> {
        let hash_hex = chunk.hash.to_hex().to_string();
        validate_object_id(&hash_hex)?;
        self.backend.put(hash_hex.as_bytes(), &chunk.data)?;
        Ok(())
    }

    pub fn get_chunk(&self, hash_hex: &str) -> Result<Vec<u8>> {
        validate_object_id(hash_hex)?;
        self.backend
            .get(hash_hex.as_bytes())?
            .ok_or_else(|| anyhow::anyhow!("Chunk not found: {}", hash_hex))
    }

    pub fn has_chunk(&self, hash_hex: &str) -> bool {
        if validate_object_id(hash_hex).is_err() {
            return false;
        }
        self.backend.contains(hash_hex.as_bytes()).unwrap_or(false)
    }

    /// Returns object IDs via key-only query (`SELECT key FROM shard_store`).
    pub fn iter_chunks(&self) -> Result<Vec<(String, String)>> {
        let keys = self.backend.iter_keys(b"")?;
        Ok(keys
            .into_iter()
            .filter_map(|k| {
                let hash = String::from_utf8(k).ok()?;
                validate_object_id(&hash).ok()?;
                Some((hash.clone(), hash))
            })
            .collect())
    }

    pub fn delete_chunk(&self, hash_hex: &str, _full_path: Option<&str>) -> Result<()> {
        validate_object_id(hash_hex)?;
        self.backend.delete(hash_hex.as_bytes())?;
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.backend.flush()
    }

    pub fn checkpoint_for_snapshot(&self) -> Result<()> {
        self.backend.checkpoint_for_snapshot()
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
        assert!(store.get_chunk("nonexistent").is_err());
        assert!(store.get_chunk(&"0".repeat(64)).is_err());
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
        let chunks = vec![fake_chunk(b"sqlite a"), fake_chunk(b"sqlite b")];
        for c in &chunks {
            store.put_chunk(c).unwrap();
        }
        let entries = store.iter_chunks().unwrap();
        assert_eq!(entries.len(), 2);
    }
}
