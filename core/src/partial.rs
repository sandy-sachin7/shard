use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub struct PartialTransfer {
    dir: PathBuf,
    #[allow(dead_code)]
    commit_id: String,
}

impl PartialTransfer {
    pub fn new(shard_dir: &Path, commit_id: &str) -> Result<Self> {
        let dir = shard_dir.join("partial").join(commit_id);
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            commit_id: commit_id.to_string(),
        })
    }

    pub fn has_chunk(&self, chunk_id: &str) -> bool {
        self.dir.join(chunk_id).exists()
    }

    pub fn save_chunk(&self, chunk_id: &str, data: &[u8]) -> Result<()> {
        fs::write(self.dir.join(chunk_id), data)?;
        Ok(())
    }

    pub fn load_chunk(&self, chunk_id: &str) -> Result<Vec<u8>> {
        Ok(fs::read(self.dir.join(chunk_id))?)
    }

    pub fn remove_chunk(&self, chunk_id: &str) -> Result<()> {
        let path = self.dir.join(chunk_id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn list_chunks(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        if self.dir.exists() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    ids.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        }
        Ok(ids)
    }

    pub fn cleanup(&self) -> Result<()> {
        if self.dir.exists() {
            fs::remove_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

pub fn list_incomplete_transfers(shard_dir: &Path) -> Result<Vec<String>> {
    let partial_dir = shard_dir.join("partial");
    if !partial_dir.exists() {
        return Ok(Vec::new());
    }
    let mut transfers = Vec::new();
    for entry in fs::read_dir(&partial_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            transfers.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(transfers)
}

pub fn remove_transfer(shard_dir: &Path, commit_id: &str) -> Result<()> {
    let partial_dir = shard_dir.join("partial").join(commit_id);
    if partial_dir.exists() {
        fs::remove_dir_all(&partial_dir)?;
    }
    Ok(())
}
