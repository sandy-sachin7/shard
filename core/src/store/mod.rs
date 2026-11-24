pub mod flat;
pub mod sled;
pub mod sqlite;

use crate::chunker::Chunk;
use anyhow::Result;
use std::path::Path;

pub enum Store {
    Flat(flat::FlatStore),
    Sled(sled::SledStore),
    Sqlite(sqlite::SqliteStore),
}

impl Store {
    pub fn new(root: &Path, backend: &str) -> Result<Self> {
        match backend {
            "flat" => Ok(Store::Flat(flat::FlatStore::new(root))),
            "sled" => Ok(Store::Sled(sled::SledStore::new(root)?)),
            "sqlite" => Ok(Store::Sqlite(sqlite::SqliteStore::new(root)?)),
            _ => anyhow::bail!("Unknown storage backend: {}", backend),
        }
    }

    pub fn open(root: &Path) -> Result<Self> {
        let config = crate::config::load_config(root)?;
        let backend = config
            .get("storage_backend")
            .map(|s| s.as_str())
            .unwrap_or("flat");
        Self::new(root, backend)
    }

    pub fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        match self {
            Store::Flat(s) => s.put_chunk(chunk),
            Store::Sled(s) => s.put_chunk(chunk),
            Store::Sqlite(s) => s.put_chunk(chunk),
        }
    }

    pub fn put_chunk_overwrite(&self, chunk: &Chunk) -> Result<()> {
        match self {
            Store::Flat(s) => s.put_chunk_overwrite(chunk),
            Store::Sled(s) => s.put_chunk_overwrite(chunk),
            Store::Sqlite(s) => s.put_chunk_overwrite(chunk),
        }
    }

    pub fn get_chunk(&self, hash_hex: &str) -> Result<Vec<u8>> {
        match self {
            Store::Flat(s) => s.get_chunk(hash_hex),
            Store::Sled(s) => s.get_chunk(hash_hex),
            Store::Sqlite(s) => s.get_chunk(hash_hex),
        }
    }

    pub fn has_chunk(&self, hash_hex: &str) -> bool {
        match self {
            Store::Flat(s) => s.has_chunk(hash_hex),
            Store::Sled(s) => s.has_chunk(hash_hex),
            Store::Sqlite(s) => s.has_chunk(hash_hex),
        }
    }

    pub fn iter_chunks(&self) -> Result<Vec<(String, String)>> {
        match self {
            Store::Flat(s) => s.iter_chunks(),
            Store::Sled(s) => s.iter_chunks(),
            Store::Sqlite(s) => s.iter_chunks(),
        }
    }

    pub fn delete_chunk(&self, hash_hex: &str, full_path: Option<&str>) -> Result<()> {
        match self {
            Store::Flat(s) => s.delete_chunk(hash_hex, full_path),
            Store::Sled(s) => s.delete_chunk(hash_hex, full_path),
            Store::Sqlite(s) => s.delete_chunk(hash_hex, full_path),
        }
    }

    pub fn flush(&self) -> Result<()> {
        match self {
            Store::Flat(s) => s.flush(),
            Store::Sled(s) => s.flush(),
            Store::Sqlite(s) => s.flush(),
        }
    }

    /// Flush and fully checkpoint backend files so an external copy of `.shard`
    /// (e.g. `hf push` or backup) captures all committed data standalone.
    pub fn checkpoint_for_snapshot(&self) -> Result<()> {
        match self {
            Store::Flat(s) => s.flush(),
            Store::Sled(s) => s.checkpoint_for_snapshot(),
            Store::Sqlite(s) => s.checkpoint_for_snapshot(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_nested_sled_store_open_same_repo_succeeds() {
        let dir = tempdir().unwrap();
        crate::init(dir.path(), "sled", "none", "fixed", None, false, false).unwrap();
        let shard_dir = dir.path().join(".shard");

        let s1 = Store::open(&shard_dir).unwrap();
        let s2 = Store::open(&shard_dir).unwrap();
        assert!(matches!(s1, Store::Sled(_)));
        assert!(matches!(s2, Store::Sled(_)));

        let data = b"shared sled chunk";
        let chunk = Chunk {
            hash: blake3::hash(data),
            data: data.to_vec(),
            offset: 0,
        };
        let id = chunk.hash.to_hex().to_string();
        s1.put_chunk(&chunk).unwrap();
        assert_eq!(s2.get_chunk(&id).unwrap(), data);
    }
}
