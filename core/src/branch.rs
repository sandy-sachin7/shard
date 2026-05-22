use anyhow::Result;
use std::fs;
use std::path::Path;

/// A branch entry: (name, tip_commit_id).
pub type BranchEntry = (String, String);

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
        let branch_path = shard_dir.join("refs").join("heads").join(branch_name);
        let commit_id = if branch_path.exists() {
            Some(fs::read_to_string(&branch_path)?.trim().to_string())
        } else {
            None
        };
        Ok((Some(branch_name.to_string()), commit_id))
    } else {
        // Bare commit id (detached HEAD)
        Ok((None, Some(head)))
    }
}

/// Set HEAD to point to a branch ("ref: refs/heads/<branch>").
pub fn set_head_branch(shard_dir: &Path, branch: &str) -> Result<()> {
    fs::write(
        shard_dir.join("HEAD"),
        format!("ref: refs/heads/{}", branch),
    )?;
    Ok(())
}

/// Set HEAD to a bare commit id (detached).
pub fn set_head_commit(shard_dir: &Path, commit_id: &str) -> Result<()> {
    fs::write(shard_dir.join("HEAD"), commit_id)?;
    Ok(())
}

/// Update a branch ref to point to a commit.
pub fn update_branch_ref(shard_dir: &Path, branch: &str, commit_id: &str) -> Result<()> {
    let branch_path = shard_dir.join("refs").join("heads").join(branch);
    fs::create_dir_all(branch_path.parent().unwrap())?;
    fs::write(&branch_path, commit_id)?;
    Ok(())
}

/// Create a new branch pointing to the given commit.
pub fn create_branch(shard_dir: &Path, name: &str, commit_id: &str) -> Result<()> {
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
    let branch_path = shard_dir.join("refs").join("heads").join(name);
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
    println!("Deleted branch '{}'", name);
    Ok(())
}

/// List all branches. Returns (current_branch, all_branches).
pub fn list_branches(shard_dir: &Path) -> Result<(Option<String>, Vec<BranchEntry>)> {
    let current = resolve_head(shard_dir)?.0;
    let refs_dir = shard_dir.join("refs").join("heads");
    if !refs_dir.exists() {
        return Ok((current, Vec::new()));
    }
    let mut branches = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&refs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let commit_id = fs::read_to_string(entry.path())?.trim().to_string();
        branches.push((name, commit_id));
    }
    Ok((current, branches))
}

/// Resolve a branch or commit string to a commit id.
/// If `name` matches a branch, returns its tip. Otherwise treats it as a commit id.
pub fn resolve_rev(shard_dir: &Path, name: &str) -> Result<String> {
    let branch_path = shard_dir.join("refs").join("heads").join(name);
    if branch_path.exists() {
        return Ok(fs::read_to_string(&branch_path)?.trim().to_string());
    }
    // Treat as bare commit id
    Ok(name.to_string())
}
