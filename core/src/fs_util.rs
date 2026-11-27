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

/// Replace `dst` with `src` in the same directory/filesystem.
/// Uses an atomic POSIX `rename(2)` on Unix. On Windows, falls back to removing
/// an existing destination before renaming (callers hold a durable journaled backup).
pub fn replace_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::rename(src, dst)
    }
    #[cfg(not(unix))]
    {
        match fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(_) if dst.exists() => {
                fs::remove_file(dst)?;
                fs::rename(src, dst)
            }
            Err(e) => Err(e),
        }
    }
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

/// Validate a repository-relative path and return its normalized path segments.
/// Rejects empty paths, NUL bytes, leading slashes, backslashes, `..`, prefixes,
/// and internal paths (`.shard`, `.git`, `.shard.lock`).
pub fn validate_relative_components(rel: &str) -> Result<Vec<std::ffi::OsString>> {
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

    let mut parts = Vec::new();
    for comp in Path::new(rel).components() {
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
                if parts.is_empty()
                    && (seg_str == ".shard" || seg_str == ".git" || seg_str == ".shard.lock")
                {
                    return Err(ShardError::PathTraversal(
                        rel.to_string(),
                        format!("access to internal path '{}' is forbidden", seg_str),
                    )
                    .into());
                }
                parts.push(seg.to_os_string());
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

    if parts.is_empty() {
        return Err(ShardError::PathTraversal(
            rel.to_string(),
            "path resolves to repository root".into(),
        )
        .into());
    }

    Ok(parts)
}

/// Descriptor-pinned repository root handle preventing TOCTOU symlink escapes.
///
/// On Unix, holds an `O_DIRECTORY | O_CLOEXEC` file descriptor for the repository root
/// and resolves every relative path component-by-component with `openat(..., O_NOFOLLOW)`,
/// `mkdirat`, `renameat`, `unlinkat`, and `fstatat(..., AT_SYMLINK_NOFOLLOW)`.
/// On non-Unix platforms (Windows), enforces canonical root containment and symlink checks.
pub struct PinnedRepoDir {
    #[cfg(unix)]
    root_fd: fs::File,
    #[cfg(not(unix))]
    root_path: PathBuf,
}

#[cfg(unix)]
mod unix_pinned {
    use super::*;
    use std::ffi::{CStr, CString, OsStr};
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};

    fn os_to_cstring(seg: &OsStr, rel: &str) -> Result<CString> {
        CString::new(seg.as_bytes()).map_err(|_| {
            ShardError::PathTraversal(rel.to_string(), "path contains NUL byte".into()).into()
        })
    }

    fn is_symlink_at(dir_fd: libc::c_int, name: &CStr) -> bool {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc =
            unsafe { libc::fstatat(dir_fd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        rc == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFLNK
    }

    fn fsync_fd(fd: libc::c_int) {
        unsafe {
            let _ = libc::fsync(fd);
        }
    }

    fn open_subdir_no_follow(
        parent_fd: libc::c_int,
        name: &CStr,
        rel: &str,
        create_missing: bool,
    ) -> Result<fs::File> {
        if create_missing {
            let mk_rc = unsafe { libc::mkdirat(parent_fd, name.as_ptr(), 0o755) };
            if mk_rc == 0 {
                fsync_fd(parent_fd);
            } else {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EEXIST) {
                    return Err(err.into());
                }
            }
        }

        let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY;
        let fd = unsafe { libc::openat(parent_fd, name.as_ptr(), flags) };
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ELOOP)
                || (err.raw_os_error() == Some(libc::ENOTDIR) && is_symlink_at(parent_fd, name))
            {
                return Err(ShardError::PathTraversal(
                    rel.to_string(),
                    "symlink directory component rejected (O_NOFOLLOW)".into(),
                )
                .into());
            }
            return Err(err.into());
        }
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }

    impl PinnedRepoDir {
        /// Open and pin `root` as an `O_DIRECTORY | O_CLOEXEC` file descriptor.
        pub fn open(root: &Path) -> Result<Self> {
            let root_fd = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(root)?;
            Ok(Self { root_fd })
        }

        fn open_parent_and_leaf(
            &self,
            rel: &str,
            create_missing: bool,
        ) -> Result<(fs::File, CString)> {
            let parts = validate_relative_components(rel)?;
            let mut cur = self.root_fd.try_clone()?;
            for seg in &parts[..parts.len() - 1] {
                let c_name = os_to_cstring(seg, rel)?;
                cur = open_subdir_no_follow(cur.as_raw_fd(), &c_name, rel, create_missing)?;
            }
            let leaf = os_to_cstring(parts.last().expect("non-empty parts"), rel)?;
            Ok((cur, leaf))
        }

        /// Check whether `rel` exists without following symlinks.
        /// Returns `Err(ShardError::PathTraversal)` if `rel` or any parent is a symlink.
        pub fn exists_no_follow(&self, rel: &str) -> Result<bool> {
            let (parent, leaf) = match self.open_parent_and_leaf(rel, false) {
                Ok(pair) => pair,
                Err(e) => {
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        if io_err.kind() == std::io::ErrorKind::NotFound {
                            return Ok(false);
                        }
                    }
                    return Err(e);
                }
            };
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let rc = unsafe {
                libc::fstatat(
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::NotFound {
                    return Ok(false);
                }
                return Err(err.into());
            }
            if (st.st_mode & libc::S_IFMT) == libc::S_IFLNK {
                return Err(ShardError::PathTraversal(
                    rel.to_string(),
                    "target path is a symlink (AT_SYMLINK_NOFOLLOW)".into(),
                )
                .into());
            }
            Ok(true)
        }

        /// Open an existing regular file at `rel` for reading with `O_NOFOLLOW`.
        pub fn open_read(&self, rel: &str) -> Result<fs::File> {
            let (parent, leaf) = self.open_parent_and_leaf(rel, false)?;
            let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
            let fd = unsafe { libc::openat(parent.as_raw_fd(), leaf.as_ptr(), flags) };
            if fd < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::ELOOP) {
                    return Err(ShardError::PathTraversal(
                        rel.to_string(),
                        "target file is a symlink (O_NOFOLLOW)".into(),
                    )
                    .into());
                }
                return Err(err.into());
            }
            let file = unsafe { fs::File::from_raw_fd(fd) };
            if !file.metadata()?.is_file() {
                return Err(ShardError::PathTraversal(
                    rel.to_string(),
                    "target path is not a regular file".into(),
                )
                .into());
            }
            Ok(file)
        }

        /// Create a new file exclusively at `rel` (`O_CREAT | O_EXCL | O_NOFOLLOW`),
        /// creating and fsyncing any missing intermediate directories via `mkdirat`.
        pub fn create_new(&self, rel: &str) -> Result<fs::File> {
            let (parent, leaf) = self.open_parent_and_leaf(rel, true)?;
            let flags =
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
            let fd = unsafe { libc::openat(parent.as_raw_fd(), leaf.as_ptr(), flags, 0o644) };
            if fd < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::ELOOP) {
                    return Err(ShardError::PathTraversal(
                        rel.to_string(),
                        "target path is a symlink (O_NOFOLLOW)".into(),
                    )
                    .into());
                }
                return Err(err.into());
            }
            Ok(unsafe { fs::File::from_raw_fd(fd) })
        }

        /// Descriptor-pinned durable copy from `src_rel` to a brand-new `dst_rel`.
        /// Syncs the written file and its parent directory, returning the Blake3 hex digest.
        pub fn copy_durable(&self, src_rel: &str, dst_rel: &str) -> Result<String> {
            let mut src = self.open_read(src_rel)?;
            let mut dst = self.create_new(dst_rel)?;
            let mut hasher = blake3::Hasher::new();
            let mut buf = [0u8; 65_536];
            loop {
                let n = src.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                dst.write_all(&buf[..n])?;
            }
            dst.sync_all()?;
            self.fsync_parent(dst_rel)?;
            Ok(hasher.finalize().to_hex().to_string())
        }

        /// Atomically replace `dst_rel` with `src_rel` inside the same pinned parent
        /// directory via `renameat`, rejecting symlinks at `dst_rel`, then fsync parent.
        pub fn replace_file(&self, src_rel: &str, dst_rel: &str) -> Result<()> {
            let src_parts = validate_relative_components(src_rel)?;
            let dst_parts = validate_relative_components(dst_rel)?;
            if src_parts[..src_parts.len() - 1] != dst_parts[..dst_parts.len() - 1] {
                anyhow::bail!(
                    "pinned replace_file requires identical parent directories ('{}' -> '{}')",
                    src_rel,
                    dst_rel
                );
            }
            let (parent, dst_leaf) = self.open_parent_and_leaf(dst_rel, true)?;
            let src_leaf = os_to_cstring(src_parts.last().expect("non-empty"), src_rel)?;
            if is_symlink_at(parent.as_raw_fd(), &dst_leaf) {
                return Err(ShardError::PathTraversal(
                    dst_rel.to_string(),
                    "refusing to overwrite symlink destination".into(),
                )
                .into());
            }
            let rc = unsafe {
                libc::renameat(
                    parent.as_raw_fd(),
                    src_leaf.as_ptr(),
                    parent.as_raw_fd(),
                    dst_leaf.as_ptr(),
                )
            };
            if rc != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            fsync_fd(parent.as_raw_fd());
            Ok(())
        }

        /// Unlink `rel` inside its pinned parent directory via `unlinkat` and fsync parent.
        /// Returns `Ok(false)` if the file was already absent.
        pub fn unlink(&self, rel: &str) -> Result<bool> {
            let (parent, leaf) = match self.open_parent_and_leaf(rel, false) {
                Ok(pair) => pair,
                Err(e) => {
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        if io_err.kind() == std::io::ErrorKind::NotFound {
                            return Ok(false);
                        }
                    }
                    return Err(e);
                }
            };
            let rc = unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), 0) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::NotFound {
                    return Ok(false);
                }
                return Err(err.into());
            }
            fsync_fd(parent.as_raw_fd());
            Ok(true)
        }

        /// Fsync the pinned parent directory of `rel`.
        pub fn fsync_parent(&self, rel: &str) -> Result<()> {
            let (parent, _) = self.open_parent_and_leaf(rel, false)?;
            fsync_fd(parent.as_raw_fd());
            Ok(())
        }
    }
}

#[cfg(not(unix))]
mod non_unix_pinned {
    use super::*;
    use std::io::Read;

    impl PinnedRepoDir {
        pub fn open(root: &Path) -> Result<Self> {
            let canonical = if root.exists() {
                fs::canonicalize(root)?
            } else {
                root.to_path_buf()
            };
            Ok(Self {
                root_path: canonical,
            })
        }

        pub fn exists_no_follow(&self, rel: &str) -> Result<bool> {
            let p = safe_repo_join(&self.root_path, rel)?;
            match fs::symlink_metadata(&p) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(ShardError::PathTraversal(
                            rel.to_string(),
                            "target path is a symlink".into(),
                        )
                        .into());
                    }
                    Ok(true)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.into()),
            }
        }

        pub fn open_read(&self, rel: &str) -> Result<fs::File> {
            let p = safe_repo_join(&self.root_path, rel)?;
            let meta = fs::symlink_metadata(&p)?;
            if meta.file_type().is_symlink() {
                return Err(ShardError::PathTraversal(
                    rel.to_string(),
                    "target file is a symlink".into(),
                )
                .into());
            }
            Ok(fs::File::open(p)?)
        }

        pub fn create_new(&self, rel: &str) -> Result<fs::File> {
            let p = safe_repo_join(&self.root_path, rel)?;
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)?;
            }
            Ok(fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(p)?)
        }

        pub fn copy_durable(&self, src_rel: &str, dst_rel: &str) -> Result<String> {
            let mut src = self.open_read(src_rel)?;
            let mut dst = self.create_new(dst_rel)?;
            let mut hasher = blake3::Hasher::new();
            let mut buf = [0u8; 65_536];
            loop {
                let n = src.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                dst.write_all(&buf[..n])?;
            }
            dst.sync_all()?;
            Ok(hasher.finalize().to_hex().to_string())
        }

        pub fn replace_file(&self, src_rel: &str, dst_rel: &str) -> Result<()> {
            let src = safe_repo_join(&self.root_path, src_rel)?;
            let dst = safe_repo_join(&self.root_path, dst_rel)?;
            if let Ok(meta) = fs::symlink_metadata(&dst) {
                if meta.file_type().is_symlink() {
                    return Err(ShardError::PathTraversal(
                        dst_rel.to_string(),
                        "refusing to overwrite symlink destination".into(),
                    )
                    .into());
                }
            }
            crate::fs_util::replace_file(&src, &dst)?;
            Ok(())
        }

        pub fn unlink(&self, rel: &str) -> Result<bool> {
            let p = safe_repo_join(&self.root_path, rel)?;
            match fs::remove_file(p) {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.into()),
            }
        }

        pub fn fsync_parent(&self, _rel: &str) -> Result<()> {
            Ok(())
        }
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
