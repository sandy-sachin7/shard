use anyhow::Result;
use std::path::{Path, PathBuf};
use std::fs;
use crate::chunker::Chunk;

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    pub fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        let hash_hex = chunk.hash.to_hex().to_string();
        let prefix = &hash_hex[..2];
        let filename = &hash_hex;

        let dir = self.root.join("objects").join(prefix);
        fs::create_dir_all(&dir)?;

        let path = dir.join(filename);
        if !path.exists() {
            fs::write(path, &chunk.data)?;
        }

        Ok(())
    }
}
