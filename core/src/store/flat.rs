use crate::chunker::Chunk;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub struct FlatStore {
    root: PathBuf,
}

impl FlatStore {
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

    pub fn has_chunk(&self, hash_hex: &str) -> bool {
        if hash_hex.len() < 2 {
            return false;
        }
        let prefix = &hash_hex[..2];
        let path = self.root.join("objects").join(prefix).join(hash_hex);
        path.exists()
    }

    /// Returns (hash, full_relative_path) pairs for all stored chunks.
    /// full_relative_path is like "ab/chunk_hash_hex" for proper deletion
    /// even when the filename doesn't match the prefix convention.
    pub fn iter_chunks(&self) -> Result<Vec<(String, String)>> {
        let objects_dir = self.root.join("objects");
        let mut chunks = Vec::new();
        if objects_dir.exists() {
            for entry in fs::read_dir(&objects_dir)? {
                let entry = entry?;
                let prefix_dir = entry.path();
                let prefix_name = entry.file_name().to_string_lossy().to_string();
                if prefix_dir.is_dir() {
                    for file_entry in fs::read_dir(&prefix_dir)? {
                        let file_entry = file_entry?;
                        let hash = file_entry.file_name().to_string_lossy().to_string();
                        let rel_path = format!("{}/{}", prefix_name, hash);
                        chunks.push((hash, rel_path));
                    }
                }
            }
        }
        Ok(chunks)
    }

    pub fn delete_chunk(&self, hash_hex: &str, full_path: Option<&str>) -> Result<()> {
        let path = if let Some(fp) = full_path {
            self.root.join("objects").join(fp)
        } else {
            if hash_hex.len() < 2 {
                anyhow::bail!("Invalid hash: {}", hash_hex);
            }
            let prefix = &hash_hex[..2];
            self.root.join("objects").join(prefix).join(hash_hex)
        };
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
