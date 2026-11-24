use crate::error::ShardError;
use crate::fs_util::{atomic_write, fsync_dir};
use anyhow::Result;
use std::fs;
use std::path::Path;

/// A branch entry: (name, tip_commit_id).
pub type BranchEntry = (String, String);

/// Validate a branch name to prevent path traversal (`../`, absolute paths, backslashes,
/// NUL bytes, control chars) while cleanly supporting hierarchical names like `feature/foo`.
pub fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(ShardError::InvalidBranchName(
            name.to_string(),
            "branch name cannot be empty".into(),
        )
        .into());
    }
    if name == "HEAD" {
        return Err(
            ShardError::InvalidBranchName(name.to_string(), "'HEAD' is reserved".into()).into(),
        );
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Err(ShardError::InvalidBranchName(
            name.to_string(),
            "leading, trailing, or consecutive slashes are not allowed".into(),
        )
        .into());
    }
    if name.contains("..") || name.contains('\\') || name.contains("@{") {
        return Err(ShardError::InvalidBranchName(
            name.to_string(),
            "contains forbidden sequence ('..', '\\', or '@{')".into(),
        )
        .into());
    }
    if name.chars().any(|c| {
        c.is_control() || c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[')
    }) {
        return Err(ShardError::InvalidBranchName(
            name.to_string(),
            "contains forbidden characters".into(),
        )
        .into());
    }
    for segment in name.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.starts_with('.')
            || segment.ends_with('.')
            || segment.ends_with(".lock")
        {
            return Err(ShardError::InvalidBranchName(
                name.to_string(),
                format!("invalid branch path segment '{}'", segment),
            )
            .into());
        }
    }
    Ok(())
}

/// Resolve the current HEAD.
/// Returns `(branch_name, commit_id)`.
/// - On a branch: `(Some("main"), Some("abc..."))` or `(Some("main"), None)` (no commits yet)
/// - Detached: `(None, Some("abc..."))`
/// - No HEAD: `(None, None)`
pub fn resolve_head(shard_dir: &Path) -> Result<(Option<String>, Option<String>)> {
    let head_path = shard_dir.join("HEAD");
    if !head_path.exists() {
        return Ok((None, None));
    }
    let head = fs::read_to_string(&head_path)?;
    let head = head.trim().to_string();

    if let Some(branch_name) = head.strip_prefix("ref: refs/heads/") {
        validate_branch_name(branch_name)?;
        let branch_path = shard_dir.join("refs").join("heads").join(branch_name);
        let commit_id = if branch_path.exists() {
            let val = fs::read_to_string(&branch_path)?.trim().to_string();
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        } else {
            None
        };
        Ok((Some(branch_name.to_string()), commit_id))
    } else if head.is_empty() {
        Ok((None, None))
    } else {
        Ok((None, Some(head)))
    }
}

/// Set HEAD to point to a branch (`ref: refs/heads/<branch>`).
pub fn set_head_branch(shard_dir: &Path, branch: &str) -> Result<()> {
    validate_branch_name(branch)?;
    atomic_write(
        &shard_dir.join("HEAD"),
        format!("ref: refs/heads/{}", branch).as_bytes(),
    )?;
    Ok(())
}

/// Set HEAD to a bare commit id (detached).
pub fn set_head_commit(shard_dir: &Path, commit_id: &str) -> Result<()> {
    atomic_write(&shard_dir.join("HEAD"), commit_id.as_bytes())?;
    Ok(())
}

/// Update a branch ref atomically to point to a commit.
pub fn update_branch_ref(shard_dir: &Path, branch: &str, commit_id: &str) -> Result<()> {
    validate_branch_name(branch)?;
    let branch_path = shard_dir.join("refs").join("heads").join(branch);
    atomic_write(&branch_path, commit_id.as_bytes())?;
    Ok(())
}

/// Create a new branch pointing to the given commit.
pub fn create_branch(shard_dir: &Path, name: &str, commit_id: &str) -> Result<()> {
    validate_branch_name(name)?;
    let branch_path = shard_dir.join("refs").join("heads").join(name);
    if branch_path.exists() {
        anyhow::bail!("Branch '{}' already exists", name);
    }
    update_branch_ref(shard_dir, name, commit_id)?;
    println!(
        "Created branch '{}' at {}",
        name,
        &commit_id[..8.min(commit_id.len())]
    );
    Ok(())
}

/// Delete a branch.
pub fn delete_branch(shard_dir: &Path, name: &str) -> Result<()> {
    validate_branch_name(name)?;
    let refs_root = shard_dir.join("refs").join("heads");
    let branch_path = refs_root.join(name);
    if !branch_path.exists() {
        anyhow::bail!("Branch '{}' not found", name);
    }
    let (current, _) = resolve_head(shard_dir)?;
    if current.as_deref() == Some(name) {
        anyhow::bail!(
            "Cannot delete branch '{}' — it is currently checked out",
            name
        );
    }
    fs::remove_file(&branch_path)?;
    // Clean up empty intermediate directories created for slashed branch names
    let mut cur = branch_path.parent();
    while let Some(dir) = cur {
        if dir == refs_root {
            fsync_dir(dir);
            break;
        }
        if fs::remove_dir(dir).is_err() {
            fsync_dir(dir);
            break;
        }
        cur = dir.parent();
    }
    println!("Deleted branch '{}'", name);
    Ok(())
}

/// List all branches recursively (including hierarchical names like `feature/foo`).
/// Returns `(current_branch, all_branches)` sorted by branch name.
pub fn list_branches(shard_dir: &Path) -> Result<(Option<String>, Vec<BranchEntry>)> {
    let current = resolve_head(shard_dir)?.0;
    let refs_dir = shard_dir.join("refs").join("heads");
    if !refs_dir.exists() {
        return Ok((current, Vec::new()));
    }
    let mut branches = Vec::new();
    collect_branches_recursive(&refs_dir, &refs_dir, &mut branches)?;
    branches.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((current, branches))
}

fn collect_branches_recursive(base: &Path, dir: &Path, out: &mut Vec<BranchEntry>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_branches_recursive(base, &path, out)?;
        } else if ft.is_file() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            // Skip any transient atomic-write temp files
            if file_name.starts_with('.') {
                continue;
            }
            let rel = path.strip_prefix(base)?;
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let commit_id = fs::read_to_string(&path)?.trim().to_string();
            out.push((name, commit_id));
        }
    }
    Ok(())
}

/// Resolve a branch or commit string to a commit id.
/// If `name` is a valid branch name and matches a ref, returns its tip.
/// Otherwise treats it as a commit id.
pub fn resolve_rev(shard_dir: &Path, name: &str) -> Result<String> {
    if validate_branch_name(name).is_ok() {
        let branch_path = shard_dir.join("refs").join("heads").join(name);
        if branch_path.is_file() {
            return Ok(fs::read_to_string(&branch_path)?.trim().to_string());
        }
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_shard(dir: &Path) {
        fs::create_dir_all(dir.join("refs/heads")).unwrap();
        set_head_branch(dir, "main").unwrap();
    }

    #[test]
    fn test_validate_branch_name() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature/foo").is_ok());
        assert!(validate_branch_name("release/v2.4-rc1").is_ok());
        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("../escape").is_err());
        assert!(validate_branch_name("feature/../../HEAD").is_err());
        assert!(validate_branch_name("/abs/branch").is_err());
        assert!(validate_branch_name("trailing/").is_err());
        assert!(validate_branch_name("double//slash").is_err());
        assert!(validate_branch_name("bad\\slash").is_err());
        assert!(validate_branch_name("HEAD").is_err());
        assert!(validate_branch_name("foo.lock").is_err());
        assert!(validate_branch_name("has space").is_err());
    }

    #[test]
    fn test_resolve_head_empty() {
        let dir = tempdir().unwrap();
        let (branch, commit) = resolve_head(dir.path()).unwrap();
        assert!(branch.is_none());
        assert!(commit.is_none());
    }

    #[test]
    fn test_set_head_branch_and_resolve() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("refs/heads")).unwrap();
        set_head_branch(dir.path(), "main").unwrap();
        update_branch_ref(dir.path(), "main", "abc123").unwrap();
        let (branch, commit) = resolve_head(dir.path()).unwrap();
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(commit.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_resolve_head_detached() {
        let dir = tempdir().unwrap();
        set_head_commit(dir.path(), "detachedhash").unwrap();
        let (branch, commit) = resolve_head(dir.path()).unwrap();
        assert!(branch.is_none());
        assert_eq!(commit.as_deref(), Some("detachedhash"));
    }

    #[test]
    fn test_resolve_rev_branch() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("refs/heads")).unwrap();
        update_branch_ref(dir.path(), "feature/foo", "featurehash").unwrap();
        let result = resolve_rev(dir.path(), "feature/foo").unwrap();
        assert_eq!(result, "featurehash");
    }

    #[test]
    fn test_resolve_rev_commit_id() {
        let dir = tempdir().unwrap();
        let result = resolve_rev(dir.path(), "abc123").unwrap();
        assert_eq!(result, "abc123");
    }

    #[test]
    fn test_create_and_delete_slashed_branch() {
        let dir = tempdir().unwrap();
        let shard = dir.path();
        init_shard(shard);
        update_branch_ref(shard, "main", "somecommit").unwrap();

        create_branch(shard, "feature/nested/branch", "testcommit").unwrap();
        assert!(create_branch(shard, "feature/nested/branch", "other").is_err());

        let (_, branches) = list_branches(shard).unwrap();
        assert!(branches
            .iter()
            .any(|(n, c)| n == "feature/nested/branch" && c == "testcommit"));

        delete_branch(shard, "feature/nested/branch").unwrap();
        assert!(delete_branch(shard, "nonexistent").is_err());
    }

    #[test]
    fn test_delete_current_branch_fails() {
        let dir = tempdir().unwrap();
        let shard = dir.path();
        init_shard(shard);
        update_branch_ref(shard, "main", "commit1").unwrap();
        let result = delete_branch(shard, "main");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("currently checked out"));
    }

    #[test]
    fn test_list_branches() {
        let dir = tempdir().unwrap();
        let shard = dir.path();
        init_shard(shard);
        update_branch_ref(shard, "main", "hash1").unwrap();
        update_branch_ref(shard, "dev", "hash2").unwrap();
        update_branch_ref(shard, "feature/x", "hash3").unwrap();

        let (current, branches) = list_branches(shard).unwrap();
        assert_eq!(current.as_deref(), Some("main"));
        assert!(branches.iter().any(|(n, _)| n == "main"));
        assert!(branches.iter().any(|(n, _)| n == "dev"));
        assert!(branches.iter().any(|(n, _)| n == "feature/x"));
    }

    #[test]
    fn test_resolve_head_branch_no_commits() {
        let dir = tempdir().unwrap();
        let shard = dir.path();
        set_head_branch(shard, "main").unwrap();
        let (branch, commit) = resolve_head(shard).unwrap();
        assert_eq!(branch.as_deref(), Some("main"));
        assert!(commit.is_none());
    }
}
