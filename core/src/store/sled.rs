use crate::chunker::Chunk;
use anyhow::Result;
use shard_storage::{open_backend, StorageBackend};
use std::path::Path;

pub struct SledStore {
    backend: Box<dyn StorageBackend>,
}

impl SledStore {
    pub fn new(root: &Path) -> Result<Self> {
        let db_path = root.join("objects.db");
        let backend = open_backend(&db_path, "sled")?;
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

    /// Returns (hash, hash) pairs since sled is key-based.
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
