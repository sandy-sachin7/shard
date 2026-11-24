use crate::branch;
use crate::fs_util::atomic_write;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::warn;

#[derive(Serialize, Deserialize, Debug)]
pub enum WalEntry {
    AddBegin {
        path: String,
    },
    AddEnd,
    CommitBegin {
        head_backup: Option<String>,
        #[serde(default)]
        branch_name: Option<String>,
        #[serde(default)]
        branch_ref_backup: Option<String>,
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

    /// Append a CRC32-checksummed WAL record and `sync_all()` to disk.
    pub fn append(&self, entry: &WalEntry) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let json = serde_json::to_string(entry)?;
        let crc = crc32fast::hash(json.as_bytes());
        writeln!(file, "{:08x}|{}", crc, json)?;
        file.sync_all()?;
        Ok(())
    }

    /// Read all valid WAL entries. Supports both CRC32-framed records and legacy
    /// plain-JSON lines. Gracefully ignores an incomplete or corrupted trailing
    /// line left by a mid-write crash instead of wedging the repository.
    pub fn read(&self) -> Result<Vec<WalEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)?;
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();

        let mut entries = Vec::with_capacity(lines.len());
        for (idx, line) in lines.iter().enumerate() {
            let is_last = idx + 1 == lines.len();
            match parse_wal_line(line) {
                Ok(entry) => entries.push(entry),
                Err(e) if is_last => {
                    warn!(
                        "Ignoring truncated/corrupted trailing WAL record during recovery: {}",
                        e
                    );
                    break;
                }
                Err(e) => {
                    warn!("Stopping WAL replay at corrupted record {}: {}", idx, e);
                    break;
                }
            }
        }
        Ok(entries)
    }

    pub fn truncate(&self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
            if let Some(parent) = self.path.parent() {
                crate::fs_util::fsync_dir(parent);
            }
        }
        Ok(())
    }
}

fn parse_wal_line(line: &str) -> Result<WalEntry> {
    if let Some((crc_hex, json)) = line.split_once('|') {
        if crc_hex.len() == 8 && crc_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            let expected = u32::from_str_radix(crc_hex, 16)?;
            let actual = crc32fast::hash(json.as_bytes());
            if expected != actual {
                anyhow::bail!(
                    "WAL CRC32 mismatch (expected {:08x}, got {:08x})",
                    expected,
                    actual
                );
            }
            return Ok(serde_json::from_str(json)?);
        }
    }
    // Legacy plain JSON line
    Ok(serde_json::from_str(line)?)
}

/// Recover from a crash by inspecting the WAL.
/// If a `CommitBegin` was written without a matching `CommitEnd`, atomically restore
/// `HEAD`, the active branch ref (`refs/heads/<branch>`), and `index`.
/// Incomplete `AddBegin` entries are cleared (orphaned chunks are cleaned by GC).
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

    let has_commit_begin = entries
        .iter()
        .any(|e| matches!(e, WalEntry::CommitBegin { .. }));
    let has_commit_end = entries.iter().any(|e| matches!(e, WalEntry::CommitEnd));

    if has_commit_begin && !has_commit_end {
        for entry in &entries {
            if let WalEntry::CommitBegin {
                head_backup,
                branch_name,
                branch_ref_backup,
                index_backup,
            } = entry
            {
                let head_path = shard_dir.join("HEAD");
                match head_backup {
                    Some(head) => atomic_write(&head_path, head.as_bytes())?,
                    None => {
                        let _ = fs::remove_file(&head_path);
                    }
                }
                if let Some(ref bname) = branch_name {
                    if branch::validate_branch_name(bname).is_ok() {
                        let ref_path = shard_dir.join("refs").join("heads").join(bname);
                        match branch_ref_backup {
                            Some(ref_val) => atomic_write(&ref_path, ref_val.as_bytes())?,
                            None => {
                                let _ = fs::remove_file(&ref_path);
                            }
                        }
                    }
                }
                atomic_write(&shard_dir.join("index"), index_backup)?;
            }
        }
        warn!("Recovered from incomplete commit (rolled back HEAD, branch ref, and index)");
    }

    wal.truncate()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_wal_append_read_roundtrip() {
        let dir = tempdir().unwrap();
        let wal = Wal::new(dir.path());

        wal.append(&WalEntry::AddBegin {
            path: "f.txt".into(),
        })
        .unwrap();
        wal.append(&WalEntry::AddEnd).unwrap();

        let entries = wal.read().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], WalEntry::AddBegin { .. }));
        assert!(matches!(entries[1], WalEntry::AddEnd));
    }

    #[test]
    fn test_wal_read_empty() {
        let dir = tempdir().unwrap();
        let wal = Wal::new(dir.path());
        assert!(!wal.exists());
        let entries = wal.read().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_wal_truncate() {
        let dir = tempdir().unwrap();
        let wal = Wal::new(dir.path());
        wal.append(&WalEntry::AddEnd).unwrap();
        assert!(wal.exists());
        wal.truncate().unwrap();
        assert!(!wal.exists());
    }

    #[test]
    fn test_wal_commit_begin_end_roundtrip() {
        let dir = tempdir().unwrap();
        let wal = Wal::new(dir.path());
        wal.append(&WalEntry::CommitBegin {
            head_backup: Some("ref: refs/heads/main".into()),
            branch_name: Some("main".into()),
            branch_ref_backup: Some("oldcommit".into()),
            index_backup: b"index_data".to_vec(),
        })
        .unwrap();
        wal.append(&WalEntry::CommitEnd).unwrap();
        let entries = wal.read().unwrap();
        assert_eq!(entries.len(), 2);
        if let WalEntry::CommitBegin {
            head_backup,
            branch_name,
            branch_ref_backup,
            index_backup,
        } = &entries[0]
        {
            assert_eq!(head_backup.as_deref(), Some("ref: refs/heads/main"));
            assert_eq!(branch_name.as_deref(), Some("main"));
            assert_eq!(branch_ref_backup.as_deref(), Some("oldcommit"));
            assert_eq!(index_backup, b"index_data");
        } else {
            panic!("Expected CommitBegin");
        }
    }

    #[test]
    fn test_recover_no_wal() {
        let dir = tempdir().unwrap();
        recover(dir.path()).unwrap();
    }

    #[test]
    fn test_recover_empty_wal() {
        let dir = tempdir().unwrap();
        let wal = Wal::new(dir.path());
        wal.append(&WalEntry::AddEnd).unwrap();
        recover(dir.path()).unwrap();
        assert!(!wal.exists());
    }

    #[test]
    fn test_recover_incomplete_commit_restores_branch_ref_and_index() {
        let dir = tempdir().unwrap();
        let shard = dir.path();
        fs::create_dir_all(shard.join("refs/heads")).unwrap();

        fs::write(shard.join("HEAD"), "ref: refs/heads/main").unwrap();
        fs::write(shard.join("refs/heads/main"), "old_commit_id").unwrap();
        fs::write(shard.join("index"), b"original_index").unwrap();

        let wal = Wal::new(shard);
        wal.append(&WalEntry::CommitBegin {
            head_backup: Some("ref: refs/heads/main".into()),
            branch_name: Some("main".into()),
            branch_ref_backup: Some("old_commit_id".into()),
            index_backup: b"original_index".to_vec(),
        })
        .unwrap();

        // Simulate crash mid-commit after updating branch ref and index
        fs::write(shard.join("refs/heads/main"), "new_commit_id").unwrap();
        fs::write(shard.join("index"), b"new_index").unwrap();

        recover(shard).unwrap();
        assert_eq!(
            fs::read_to_string(shard.join("HEAD")).unwrap(),
            "ref: refs/heads/main"
        );
        assert_eq!(
            fs::read_to_string(shard.join("refs/heads/main")).unwrap(),
            "old_commit_id"
        );
        assert_eq!(fs::read(shard.join("index")).unwrap(), b"original_index");
    }

    #[test]
    fn test_recover_truncated_trailing_wal_line_does_not_wedge() {
        let dir = tempdir().unwrap();
        let shard = dir.path();
        let wal = Wal::new(shard);
        wal.append(&WalEntry::CommitBegin {
            head_backup: Some("old_head".into()),
            branch_name: None,
            branch_ref_backup: None,
            index_backup: b"orig_idx".to_vec(),
        })
        .unwrap();
        // Append truncated garbage simulating power loss mid-write of CommitEnd
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(shard.join("wal.log"))
            .unwrap();
        f.write_all(b"deadbeef|{\"CommitEn").unwrap();

        fs::write(shard.join("HEAD"), "corrupt_head").unwrap();
        fs::write(shard.join("index"), b"bad_idx").unwrap();

        recover(shard).unwrap();
        assert_eq!(fs::read_to_string(shard.join("HEAD")).unwrap(), "old_head");
        assert_eq!(fs::read(shard.join("index")).unwrap(), b"orig_idx");
        assert!(!wal.exists());
    }

    #[test]
    fn test_recover_incomplete_add() {
        let dir = tempdir().unwrap();
        let wal = Wal::new(dir.path());
        wal.append(&WalEntry::AddBegin {
            path: "f.txt".into(),
        })
        .unwrap();
        recover(dir.path()).unwrap();
        assert!(!wal.exists());
    }
}
