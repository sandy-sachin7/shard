pub mod flat;
pub mod sled;
pub mod sqlite;

use crate::chunker::Chunk;
use crate::metadata;
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
        let config_path = root.join("config.json");
        let backend = if config_path.exists() {
            let data = std::fs::read(&config_path)?;
            let config: std::collections::BTreeMap<String, String> = metadata::deserialize(&data)?;
            config
                .get("storage_backend")
                .map(|s| s.as_str())
                .unwrap_or("flat")
                .to_string()
        } else {
            "flat".to_string()
        };
        Self::new(root, &backend)
    }

    pub fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        match self {
            Store::Flat(s) => s.put_chunk(chunk),
            Store::Sled(s) => s.put_chunk(chunk),
            Store::Sqlite(s) => s.put_chunk(chunk),
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
}
