use crate::error::ShardError;
use anyhow::Result;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Validate that `id` is a strict 64-character lowercase hexadecimal Blake3 digest.
/// Prevents path traversal and non-UTF-8 boundary slicing panics.
pub fn validate_object_id(id: &str) -> Result<()> {
    if id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(ShardError::InvalidObjectId(id.to_string()).into())
    }
}

/// Safely join a relative repository path `rel` onto `root`, preventing:
/// - empty paths or NUL bytes
/// - absolute paths, Windows prefixes, or `..` traversal components
/// - writes into `.shard`, `.shard.lock`, or `.git` internal paths
/// - symlink escapes outside `root`
pub fn safe_repo_join(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Err(ShardError::PathTraversal(rel.to_string(), "path is empty".into()).into());
    }
    if rel.contains('\0') {
        return Err(
            ShardError::PathTraversal(rel.to_string(), "path contains NUL byte".into()).into(),
        );
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(ShardError::PathTraversal(
            rel.to_string(),
            "absolute paths are forbidden".into(),
        )
        .into());
    }

    let rel_path = Path::new(rel);
    let mut normalized = PathBuf::new();
    let mut first_component = true;

    for comp in rel_path.components() {
        match comp {
            Component::Normal(seg) => {
                let seg_str = seg.to_string_lossy();
                if seg_str.contains('\\') {
                    return Err(ShardError::PathTraversal(
                        rel.to_string(),
                        "backslashes are not permitted in repository paths".into(),
                    )
                    .into());
                }
                if first_component
                    && (seg_str == ".shard" || seg_str == ".git" || seg_str == ".shard.lock")
                {
                    return Err(ShardError::PathTraversal(
                        rel.to_string(),
                        format!("access to internal path '{}' is forbidden", seg_str),
                    )
                    .into());
                }
                first_component = false;
                normalized.push(seg);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ShardError::PathTraversal(
                    rel.to_string(),
                    "parent directory traversal ('..') is forbidden".into(),
                )
                .into());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ShardError::PathTraversal(
                    rel.to_string(),
                    "absolute or prefixed paths are forbidden".into(),
                )
                .into());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ShardError::PathTraversal(
            rel.to_string(),
            "path resolves to repository root".into(),
        )
        .into());
    }

    let canonical_root = if root.exists() {
        fs::canonicalize(root)?
    } else {
        root.to_path_buf()
    };

    // Walk existing path prefix component-by-component to catch symlinks escaping `root`.
    let mut probe = root.to_path_buf();
    for comp in normalized.components() {
        probe.push(comp);
        if let Ok(meta) = fs::symlink_metadata(&probe) {
            if meta.file_type().is_symlink() {
                if let Ok(resolved) = fs::canonicalize(&probe) {
                    if !resolved.starts_with(&canonical_root) {
                        return Err(ShardError::PathTraversal(
                            rel.to_string(),
                            "symlink escapes repository root".into(),
                        )
                        .into());
                    }
                } else {
                    // Dangling symlink could point anywhere once created — refuse it.
                    return Err(ShardError::PathTraversal(
                        rel.to_string(),
                        "dangling symlink in target path".into(),
                    )
                    .into());
                }
            }
        }
    }

    let joined = root.join(&normalized);
    if joined.exists() {
        let canonical_target = fs::canonicalize(&joined)?;
        if !canonical_target.starts_with(&canonical_root) {
            return Err(ShardError::PathTraversal(
                rel.to_string(),
                "resolved path escapes repository root".into(),
            )
            .into());
        }
    }

    Ok(joined)
}

/// Atomically write `data` to `path`:
/// write to same-directory temp file -> `sync_all()` -> cross-platform `persist()` -> parent dir fsync.
/// Uses `tempfile::NamedTempFile::persist` so overwriting an existing target works
/// atomically on both Unix (`rename`) and Windows (`MoveFileExW(MOVEFILE_REPLACE_EXISTING)`).
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("target path '{}' has no parent", path.display()))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("shard");

    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{}.tmp.", name))
        .tempfile_in(parent)?;
    tmp.as_file_mut().write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;

    fsync_dir(parent);
    Ok(())
}

/// Atomically write sensitive bytes with `0o600` permissions on Unix.
pub fn atomic_write_secret(path: &Path, data: &[u8]) -> Result<()> {
    shard_crypto::write_secret_file(path, data)
}

/// Set `0o600` permissions on a secret file (Unix only).
pub fn set_secret_permissions(path: &Path) -> Result<()> {
    shard_crypto::set_secret_permissions(path)
}

/// Best-effort fsync of a directory on Unix so renames are durable across power loss.
pub fn fsync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_validate_object_id() {
        let valid = "a".repeat(64);
        assert!(validate_object_id(&valid).is_ok());
        assert!(validate_object_id("0123456789abcdef".repeat(4).as_str()).is_ok());
        assert!(validate_object_id("").is_err());
        assert!(validate_object_id("/etc/passwd").is_err());
        assert!(validate_object_id("../../../keys/secret.key").is_err());
        assert!(validate_object_id(&"A".repeat(64)).is_err());
        assert!(validate_object_id("éé").is_err());
    }

    #[test]
    fn test_safe_repo_join_valid_and_rejects_escapes() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        assert_eq!(
            safe_repo_join(root, "models/v1/weights.bin").unwrap(),
            root.join("models/v1/weights.bin")
        );
        assert_eq!(
            safe_repo_join(root, "./file.txt").unwrap(),
            root.join("file.txt")
        );

        assert!(safe_repo_join(root, "").is_err());
        assert!(safe_repo_join(root, "/etc/passwd").is_err());
        assert!(safe_repo_join(root, "../escape.txt").is_err());
        assert!(safe_repo_join(root, "a/../../escape.txt").is_err());
        assert!(safe_repo_join(root, ".shard/keys/secret.key").is_err());
        assert!(safe_repo_join(root, ".git/config").is_err());
        assert!(safe_repo_join(root, "foo\0bar").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_safe_repo_join_rejects_symlink_escape() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let link = root.path().join("escape_dir");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(safe_repo_join(root.path(), "escape_dir/pwned.txt").is_err());
    }

    #[test]
    fn test_atomic_write_roundtrip() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("nested/data.bin");
        atomic_write(&target, b"v1").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"v1");
        atomic_write(&target, b"v2-updated").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"v2-updated");
    }
}
