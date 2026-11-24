use crate::fs_util::{atomic_write, fsync_dir, validate_object_id};
use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PIN_DIR_NAME: &str = "inbound-pins";
pub(crate) const INBOUND_PIN_RETENTION_SECS: u64 = 24 * 60 * 60;

fn pin_dir(shard_dir: &Path) -> PathBuf {
    shard_dir.join(PIN_DIR_NAME)
}

fn pin_path(shard_dir: &Path, object_id: &str) -> PathBuf {
    pin_dir(shard_dir).join(object_id)
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Pin an inbound object for 24 hours. The caller must hold the repo write lock.
pub(crate) fn pin_object(shard_dir: &Path, object_id: &str) -> Result<()> {
    pin_object_at(shard_dir, object_id, unix_time_secs())
}

fn pin_object_at(shard_dir: &Path, object_id: &str, now: u64) -> Result<()> {
    validate_object_id(object_id)?;
    let expires_at = now.saturating_add(INBOUND_PIN_RETENTION_SECS);
    let directory_was_missing = !pin_dir(shard_dir).exists();
    atomic_write(
        &pin_path(shard_dir, object_id),
        expires_at.to_string().as_bytes(),
    )
    .with_context(|| format!("failed to write inbound pin for object {object_id}"))?;
    if directory_was_missing {
        // `atomic_write` syncs the pin file and its directory; also sync the
        // parent so the new directory entry survives a crash.
        fsync_dir(shard_dir);
    }
    Ok(())
}

/// Return live inbound pins and remove expired records.
pub(crate) fn active_object_ids(shard_dir: &Path) -> Result<HashSet<String>> {
    active_object_ids_at(shard_dir, unix_time_secs())
}

fn active_object_ids_at(shard_dir: &Path, now: u64) -> Result<HashSet<String>> {
    let directory = pin_dir(shard_dir);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(anyhow!(
                "inbound pin path is not a directory: {}",
                directory.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashSet::new());
        }
        Err(error) => return Err(error.into()),
    }

    let mut active = HashSet::new();
    let mut removed_expired = false;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            return Err(anyhow!(
                "unexpected non-file entry in inbound pin directory: {}",
                entry.path().display()
            ));
        }

        let object_id = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("inbound pin filename is not valid UTF-8"))?;
        // A crash between temp creation and rename leaves an `atomic_write`
        // temp file here. Ignore it so one interrupted pin cannot block GC.
        if object_id.starts_with('.') {
            continue;
        }
        validate_object_id(&object_id)?;
        let path = entry.path();
        let expires_at: u64 = std::str::from_utf8(&fs::read(&path)?)?
            .parse()
            .with_context(|| format!("invalid expiry in inbound pin {}", path.display()))?;
        if expires_at > now {
            active.insert(object_id);
        } else {
            fs::remove_file(path)?;
            removed_expired = true;
        }
    }

    if removed_expired {
        fsync_dir(&directory);
    }
    Ok(active)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn expired_pins_are_removed_from_disk() {
        let temp = tempdir().unwrap();
        let shard_dir = temp.path().join(".shard");
        fs::create_dir_all(&shard_dir).unwrap();
        let object_id = "a".repeat(64);

        pin_object_at(&shard_dir, &object_id, 100).unwrap();
        assert!(active_object_ids_at(&shard_dir, 101)
            .unwrap()
            .contains(&object_id));
        assert!(
            active_object_ids_at(&shard_dir, 100 + INBOUND_PIN_RETENTION_SECS)
                .unwrap()
                .is_empty()
        );
        assert!(!pin_path(&shard_dir, &object_id).exists());
    }

    #[test]
    fn crash_left_temp_file_does_not_block_active_pin_scan() {
        let temp = tempdir().unwrap();
        let shard_dir = temp.path().join(".shard");
        fs::create_dir_all(pin_dir(&shard_dir)).unwrap();
        let object_id = "a".repeat(64);

        pin_object_at(&shard_dir, &object_id, 100).unwrap();
        fs::write(pin_dir(&shard_dir).join(".a.tmp.123.0"), b"").unwrap();

        let active = active_object_ids_at(&shard_dir, 101).unwrap();
        assert!(active.contains(&object_id));
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn pin_update_writes_one_record_per_object() {
        let temp = tempdir().unwrap();
        let shard_dir = temp.path().join(".shard");
        let first = "a".repeat(64);
        let second = "b".repeat(64);

        pin_object_at(&shard_dir, &first, 100).unwrap();
        pin_object_at(&shard_dir, &second, 100).unwrap();

        let records: Vec<_> = fs::read_dir(pin_dir(&shard_dir))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(records.len(), 2);
        assert!(active_object_ids_at(&shard_dir, 101)
            .unwrap()
            .contains(&first));
        assert!(active_object_ids_at(&shard_dir, 101)
            .unwrap()
            .contains(&second));
    }
}
