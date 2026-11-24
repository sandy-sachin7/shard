use crate::error::ShardError;
use crate::fs_util::{atomic_write, validate_object_id};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Validate a transfer identifier so it cannot traverse outside `.shard/partial/`.
fn validate_transfer_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
        || id.contains("..")
    {
        return Err(ShardError::PathTraversal(
            id.to_string(),
            "invalid partial transfer identifier".into(),
        )
        .into());
    }
    Ok(())
}

pub struct PartialTransfer {
    dir: PathBuf,
    #[allow(dead_code)]
    commit_id: String,
}

impl PartialTransfer {
    pub fn new(shard_dir: &Path, commit_id: &str) -> Result<Self> {
        validate_object_id(commit_id)?;
        let dir = shard_dir.join("partial").join(commit_id);
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            commit_id: commit_id.to_string(),
        })
    }

    pub fn has_chunk(&self, chunk_id: &str) -> bool {
        if validate_object_id(chunk_id).is_err() {
            return false;
        }
        self.dir.join(chunk_id).exists()
    }

    pub fn save_chunk(&self, chunk_id: &str, data: &[u8]) -> Result<()> {
        validate_object_id(chunk_id)?;
        atomic_write(&self.dir.join(chunk_id), data)?;
        Ok(())
    }

    pub fn load_chunk(&self, chunk_id: &str) -> Result<Vec<u8>> {
        validate_object_id(chunk_id)?;
        Ok(fs::read(self.dir.join(chunk_id))?)
    }

    pub fn remove_chunk(&self, chunk_id: &str) -> Result<()> {
        validate_object_id(chunk_id)?;
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
                    let name = entry.file_name().to_string_lossy().to_string();
                    if validate_object_id(&name).is_ok() {
                        ids.push(name);
                    }
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

/// Collect all valid object IDs (commit IDs and chunk IDs) referenced by incomplete partial transfers.
pub fn collect_partial_object_ids(shard_dir: &Path) -> Result<Vec<String>> {
    let partial_dir = shard_dir.join("partial");
    let mut ids = Vec::new();
    if !partial_dir.exists() {
        return Ok(ids);
    }
    for entry in fs::read_dir(&partial_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let commit_id = entry.file_name().to_string_lossy().to_string();
        if validate_object_id(&commit_id).is_ok() {
            ids.push(commit_id);
        }
        for chunk_entry in fs::read_dir(entry.path())? {
            let chunk_entry = chunk_entry?;
            if chunk_entry.file_type()?.is_file() {
                let chunk_id = chunk_entry.file_name().to_string_lossy().to_string();
                if validate_object_id(&chunk_id).is_ok() {
                    ids.push(chunk_id);
                }
            }
        }
    }
    Ok(ids)
}

pub fn remove_transfer(shard_dir: &Path, commit_id: &str) -> Result<()> {
    validate_transfer_id(commit_id)?;
    let partial_dir = shard_dir.join("partial").join(commit_id);
    if partial_dir.exists() {
        fs::remove_dir_all(&partial_dir)?;
    }
    Ok(())
}
