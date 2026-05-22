use crate::chunker::Chunk;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        let hash_hex = chunk.hash.to_hex().to_string();
        let prefix = hash_hex.get(..2).unwrap_or("xx");
        let filename = &hash_hex;

        let dir = self.root.join("objects").join(prefix);
        fs::create_dir_all(&dir)?;

        let path = dir.join(filename);
        if !path.exists() {
            fs::write(path, &chunk.data)?;
        }

        Ok(())
    }

    pub fn get_chunk(&self, hash_hex: &str) -> Result<Vec<u8>> {
        if hash_hex.len() < 2 {
            anyhow::bail!("Invalid hash: {}", hash_hex);
        }
        let prefix = &hash_hex[..2];
        let filename = hash_hex;
        let path = self.root.join("objects").join(prefix).join(filename);

        if !path.exists() {
            anyhow::bail!("Chunk not found: {}", hash_hex);
        }

        Ok(fs::read(path)?)
    }
}
