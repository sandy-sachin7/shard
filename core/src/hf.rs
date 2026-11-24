use crate::error::ShardError;
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Sensitive file/directory names that must NEVER be uploaded to HuggingFace.
const FORBIDDEN_HF_NAMES: &[&str] = &[
    "secret.key",
    "repo.key",
    "archive",
    "wal.log",
    "partial",
    "repo.lock",
    ".shard.lock",
];

/// Stage only safe public repository artifacts from `shard_dir` into `staging_dir`
/// and run a mandatory pre-flight assertion guaranteeing no private keys, WAL files,
/// partial transfers, or lock files exist anywhere in `staging_dir`.
pub fn stage_hf_export(shard_dir: &Path, staging_dir: &Path) -> Result<()> {
    fs::create_dir_all(staging_dir)?;

    // Top-level safe files
    for file in ["HEAD", "config.json", "tags.json", "peers.json"] {
        let src = shard_dir.join(file);
        if src.is_file() {
            fs::copy(&src, staging_dir.join(file))?;
        }
    }

    // Safe directories (objects/, refs/)
    for dir in ["objects", "refs"] {
        let src = shard_dir.join(dir);
        if src.is_dir() {
            copy_dir_filtered(&src, &staging_dir.join(dir))?;
        }
    }

    // Object database for Sled (directory) or SQLite (file) backends
    let objects_db = shard_dir.join("objects.db");
    if objects_db.is_dir() {
        copy_dir_filtered(&objects_db, &staging_dir.join("objects.db"))?;
    } else if objects_db.is_file() {
        fs::copy(&objects_db, staging_dir.join("objects.db"))?;
    }

    // Public-only key metadata (NEVER secret.key, repo.key, or archive/)
    let keys_src = shard_dir.join("keys");
    if keys_src.is_dir() {
        let keys_dst = staging_dir.join("keys");
        fs::create_dir_all(&keys_dst)?;
        for pub_file in ["public.key", "current"] {
            let src = keys_src.join(pub_file);
            if src.is_file() {
                fs::copy(&src, keys_dst.join(pub_file))?;
            }
        }
        for pub_dir in ["records", "rotations"] {
            let src = keys_src.join(pub_dir);
            if src.is_dir() {
                copy_dir_filtered(&src, &keys_dst.join(pub_dir))?;
            }
        }
    }

    // Pre-flight assertion: refuse upload if ANY forbidden file/dir exists in staging tree
    assert_no_secrets_staged(staging_dir)?;
    Ok(())
}

fn copy_dir_filtered(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if FORBIDDEN_HF_NAMES.contains(&name_str.as_ref()) || name_str.starts_with('.') {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_filtered(&src_path, &dst_path)?;
        } else if ft.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn assert_no_secrets_staged(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if FORBIDDEN_HF_NAMES.contains(&name.as_str()) {
            anyhow::bail!(
                "SECURITY ABORT: forbidden file/directory '{}' detected in HuggingFace staging tree",
                entry.path().display()
            );
        }
        if entry.file_type()?.is_dir() {
            assert_no_secrets_staged(&entry.path())?;
        }
    }
    Ok(())
}

pub fn hf_push(path: &Path, dataset_id: &str, token: Option<&str>, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        return Err(ShardError::NotARepo.into());
    }

    let staging = tempfile::tempdir()?;
    let staged_shard = staging.path().join(".shard");
    crate::with_read_op(path, "hf_push_stage", || {
        stage_hf_export(&shard_dir, &staged_shard)
    })?;

    if !json {
        println!(
            "Pushing sanitized .shard (private keys excluded) to HuggingFace dataset: {}",
            dataset_id
        );
    }

    let mut cmd = std::process::Command::new("huggingface-cli");
    cmd.arg("upload");
    cmd.arg(dataset_id);
    cmd.arg(&staged_shard);
    cmd.arg(".shard");
    cmd.arg("--repo-type").arg("dataset");

    // Pass token via environment variable instead of CLI arg so it never appears in `ps`
    if let Some(t) = token.filter(|s| !s.is_empty()) {
        cmd.env("HF_TOKEN", t);
    }

    let status = cmd
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to run huggingface-cli: {}", e))?;
    if !status.success() {
        anyhow::bail!("huggingface-cli upload failed with status: {}", status);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({"status": "hf_pushed", "dataset": dataset_id})
            )?
        );
    } else {
        println!("Successfully pushed to HuggingFace!");
    }

    Ok(())
}

pub fn hf_pull(path: &Path, dataset_id: &str, token: Option<&str>, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!("repository already exists at this path");
    }

    if !json {
        println!("Pulling .shard from HuggingFace dataset: {}", dataset_id);
    }

    let mut cmd = std::process::Command::new("huggingface-cli");
    cmd.arg("download");
    cmd.arg(dataset_id);
    cmd.arg(".shard");
    cmd.arg("--local-dir").arg(path);
    cmd.arg("--repo-type").arg("dataset");

    if let Some(t) = token.filter(|s| !s.is_empty()) {
        cmd.env("HF_TOKEN", t);
    }

    let status = cmd
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to run huggingface-cli: {}", e))?;
    if !status.success() {
        anyhow::bail!("huggingface-cli download failed with status: {}", status);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({"status": "hf_pulled", "dataset": dataset_id})
            )?
        );
    } else {
        println!("Successfully pulled from HuggingFace!");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_hf_staging_excludes_private_keys_and_internal_files() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        crate::init(repo, "flat", "zstd", "fixed", None, true, false).unwrap();
        fs::write(repo.join("model.bin"), b"weights").unwrap();
        crate::add(repo, &repo.join("model.bin"), false).unwrap();
        crate::commit(repo, "init", "Alice", false).unwrap();
        crate::key_rotate(repo).unwrap();

        // Plant wal.log, partial/, and repo.lock to verify exclusion
        let shard_dir = repo.join(".shard");
        fs::write(shard_dir.join("wal.log"), b"wal").unwrap();
        fs::create_dir_all(shard_dir.join("partial/abc")).unwrap();
        fs::write(shard_dir.join("partial/abc/chunk"), b"part").unwrap();
        fs::write(shard_dir.join("repo.lock"), b"").unwrap();

        let staging = tempdir().unwrap();
        let out = staging.path().join(".shard");
        stage_hf_export(&shard_dir, &out).unwrap();

        // Public repo artifacts MUST exist
        assert!(out.join("HEAD").exists());
        assert!(out.join("config.json").exists());
        assert!(out.join("objects").exists());
        assert!(out.join("refs").exists());
        assert!(out.join("keys/public.key").exists());
        assert!(out.join("keys/current").exists());
        assert!(out.join("keys/records").exists());
        assert!(out.join("keys/rotations").exists());

        // Private keys & runtime files MUST NOT exist
        assert!(!out.join("keys/secret.key").exists());
        assert!(!out.join("keys/repo.key").exists());
        assert!(!out.join("keys/archive").exists());
        assert!(!out.join("wal.log").exists());
        assert!(!out.join("partial").exists());
        assert!(!out.join("repo.lock").exists());
        assert!(!out.join(".shard.lock").exists());
    }

    #[test]
    fn test_hf_staging_includes_objects_db_for_sled_and_sqlite_and_blocks_secrets() {
        for backend in ["sled", "sqlite"] {
            let dir = tempdir().unwrap();
            let repo = dir.path();
            crate::init(repo, backend, "zstd", "fixed", None, true, false).unwrap();
            fs::write(repo.join("weights.bin"), b"tensor weights").unwrap();
            crate::add(repo, &repo.join("weights.bin"), false).unwrap();
            crate::commit(repo, "commit weights", "Alice", false).unwrap();

            let shard_dir = repo.join(".shard");
            // Plant forbidden lock/WAL files to ensure they are excluded
            fs::write(shard_dir.join(".shard.lock"), b"").unwrap();
            fs::write(shard_dir.join("wal.log"), b"wal").unwrap();
            if backend == "sled" {
                // Even if a rogue secret file somehow sat inside objects.db/, it must be skipped
                fs::write(shard_dir.join("objects.db/secret.key"), b"nope").unwrap();
            }

            let staging = tempdir().unwrap();
            let out = staging.path().join(".shard");
            stage_hf_export(&shard_dir, &out).unwrap();

            if backend == "sled" {
                assert!(out.join("objects.db").is_dir());
                assert!(!out.join("objects.db/secret.key").exists());
            } else {
                assert!(out.join("objects.db").is_file());
                assert!(fs::metadata(out.join("objects.db")).unwrap().len() > 0);
            }

            assert!(!out.join("keys/secret.key").exists());
            assert!(!out.join("keys/repo.key").exists());
            assert!(!out.join("wal.log").exists());
            assert!(!out.join(".shard.lock").exists());

            // Pre-flight assertion must reject any forbidden file if present
            fs::write(out.join(".shard.lock"), b"").unwrap();
            assert!(assert_no_secrets_staged(&out).is_err());
        }
    }
}
