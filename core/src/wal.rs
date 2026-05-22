use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug)]
pub enum WalEntry {
    AddBegin {
        path: String,
    },
    AddEnd,
    CommitBegin {
        head_backup: Option<String>,
        index_backup: Vec<u8>,
    },
    CommitEnd,
}

pub struct Wal {
    path: PathBuf,
}

impl Wal {
    pub fn new(shard_dir: &Path) -> Self {
        Self {
            path: shard_dir.join("wal.log"),
        }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn append(&self, entry: &WalEntry) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{}", line)?;
        file.flush()?;
        Ok(())
    }

    pub fn read(&self) -> Result<Vec<WalEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)?;
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(Into::into))
            .collect()
    }

    pub fn truncate(&self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

/// Recover from a crash by inspecting the WAL.
/// If a CommitBegin was written without CommitEnd, restore HEAD and index.
/// If an AddBegin was written without AddEnd, just clear the WAL (orphaned
/// chunks will be cleaned up by `prune`).
pub fn recover(shard_dir: &Path) -> Result<()> {
    let wal = Wal::new(shard_dir);
    if !wal.exists() {
        return Ok(());
    }

    let entries = wal.read()?;
    if entries.is_empty() {
        wal.truncate()?;
        return Ok(());
    }

    // Check for incomplete commit
    let has_commit_begin = entries
        .iter()
        .any(|e| matches!(e, WalEntry::CommitBegin { .. }));
    let has_commit_end = entries.iter().any(|e| matches!(e, WalEntry::CommitEnd));

    if has_commit_begin && !has_commit_end {
        // Crash during commit — restore HEAD and index
        for entry in &entries {
            if let WalEntry::CommitBegin {
                head_backup,
                index_backup,
            } = entry
            {
                let head_path = shard_dir.join("HEAD");
                match head_backup {
                    Some(head) => fs::write(&head_path, head)?,
                    None => {
                        let _ = fs::remove_file(&head_path);
                    }
                }
                fs::write(shard_dir.join("index"), index_backup)?;
            }
        }
        eprintln!("Recovered from incomplete commit (rolled back)");
    }

    // Incomplete add: index was never saved, just clean WAL
    wal.truncate()?;
    Ok(())
}
