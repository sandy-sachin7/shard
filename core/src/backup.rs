use crate::auth;
use crate::chunker;
use crate::compression::Compression;
use crate::error::ShardError;
use crate::fs_util::validate_object_id;
use crate::index::Index;
use crate::manifest::FileManifest;
use crate::metadata::{self, MetadataFormat};
use crate::repo;
use crate::store::Store;
use anyhow::Result;
use std::fs;
use std::path::{Component, Path};

pub fn backup(path: &Path, output: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_backup
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "backup", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let file = fs::File::create(output)?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        archive.append_dir_all(".shard", &shard_dir)?;
        let encoder = archive.into_inner()?;
        let file = encoder.finish()?;
        file.sync_all()?;

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "path": output.to_string_lossy(),
                }))?
            );
        } else {
            println!("Backup created: {}", output.display());
        }
        Ok(())
    })
}

pub fn export(path: &Path, commit_id: &str, output_dir: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_export
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "export", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let store = Store::open(&shard_dir)?;
        let cipher = auth::maybe_load_cipher(&shard_dir)?;
        let commit = repo::load_commit(&store, commit_id)?;
        fs::create_dir_all(output_dir)?;

        let mut files = Vec::new();
        for manifest_id in &commit.manifests {
            validate_object_id(manifest_id)?;
            let data = store.get_chunk(manifest_id)?;
            let manifest: FileManifest = metadata::deserialize(&data)?;
            if !json {
                println!("Exporting file: {}", manifest.name);
            }
            let out_path =
                repo::stream_manifest_to_file(&store, &manifest, output_dir, cipher.as_ref())?;
            if !json {
                println!("  -> {}", out_path.display());
            }
            files.push(manifest.name);
        }

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "commit_id": commit_id,
                    "files": files,
                    "output_dir": output_dir.to_string_lossy(),
                }))?
            );
        } else {
            println!("Export complete.");
        }
        Ok(())
    })
}

pub fn import(
    path: &Path,
    source_dir: &Path,
    message: &str,
    author: &str,
    json: bool,
) -> Result<()> {
    crate::metrics::METRICS
        .ops_import
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "import", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        if !source_dir.is_dir() {
            anyhow::bail!("Source must be a directory");
        }
        crate::wal::recover(&shard_dir)?;

        let config = crate::load_config(&shard_dir)?;
        let compression: Compression = config
            .get("compression")
            .map(|s| s.as_str())
            .unwrap_or("zstd")
            .parse()?;
        let chunker_mode = chunker::ChunkerMode::from_config(&config);
        let fmt = MetadataFormat::from_config(&config);
        let store = Store::open(&shard_dir)?;
        let cipher = auth::maybe_load_cipher(&shard_dir)?;
        let mut index = Index::load(&shard_dir.join("index"), &fmt)?;

        let include_hidden = config
            .get("hidden_files_enabled")
            .map(|s| s.as_str() == "true")
            .unwrap_or(false);
        let mut builder = ignore::WalkBuilder::new(source_dir);
        builder.hidden(!include_hidden);
        builder.filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".shard" && name != ".git" && name != ".shard.lock"
        });

        for entry in builder.build() {
            let entry = entry.map_err(|e| anyhow::anyhow!("{}", e))?;
            if entry.file_type().is_some_and(|ft| ft.is_file()) {
                // Pass `source_dir` as the base root so nested subdirectories are preserved!
                repo::add_file(
                    source_dir,
                    entry.path(),
                    &store,
                    &mut index,
                    &compression,
                    &chunker_mode,
                    cipher.as_ref(),
                    json,
                )?;
            }
        }
        store.flush()?;
        index.save(&shard_dir.join("index"), &fmt)?;

        if !index.files.is_empty() {
            // Reuse the open `store` so Sled never fails with an exclusive-lock double-open!
            repo::commit_with_store(path, &shard_dir, &store, message, author, json)?;
        } else if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"status": "no files found"}))?
            );
        } else {
            println!("No files found to import.");
        }
        Ok(())
    })
}

pub fn restore(path: &Path, backup_file: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_restore
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "restore", || {
        let shard_dir = path.join(".shard");
        if shard_dir.exists() {
            anyhow::bail!(
                "Repository already exists — remove .shard first or use a different directory"
            );
        }

        let file = fs::File::open(backup_file)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);

        // Safely unpack entries validating against tar-slip path traversal and
        // strictly confining every entry inside `.shard/`.
        let unpack_result: Result<()> = (|| {
            for entry_res in archive.entries()? {
                let mut entry = entry_res?;
                let entry_path = entry.path()?.into_owned();
                if entry_path.is_absolute()
                    || entry_path.components().any(|c| {
                        matches!(
                            c,
                            Component::ParentDir | Component::RootDir | Component::Prefix(_)
                        )
                    })
                {
                    return Err(ShardError::PathTraversal(
                        entry_path.display().to_string(),
                        "unsafe path in backup archive".into(),
                    )
                    .into());
                }
                let first_normal = entry_path.components().find_map(|c| match c {
                    Component::Normal(os) => Some(os),
                    _ => None,
                });
                if first_normal != Some(std::ffi::OsStr::new(".shard")) {
                    return Err(ShardError::PathTraversal(
                        entry_path.display().to_string(),
                        "backup archive entries must be confined strictly inside .shard/".into(),
                    )
                    .into());
                }
                let entry_type = entry.header().entry_type();
                if entry_type.is_symlink() || entry_type.is_hard_link() {
                    return Err(ShardError::PathTraversal(
                        entry_path.display().to_string(),
                        "links are not permitted in backup archives".into(),
                    )
                    .into());
                }
                if !(entry_type.is_file() || entry_type.is_dir()) {
                    return Err(ShardError::PathTraversal(
                        entry_path.display().to_string(),
                        "unsupported special file type in backup archive".into(),
                    )
                    .into());
                }
                entry.unpack_in(path)?;
            }
            Ok(())
        })();

        if let Err(err) = unpack_result {
            let _ = fs::remove_dir_all(&shard_dir);
            return Err(err);
        }

        if !path.join(".shard").exists() {
            anyhow::bail!("Backup does not contain a valid .shard directory");
        }
        // Re-apply restrictive permissions on restored secret keys
        let keys_dir = path.join(".shard/keys");
        if keys_dir.join("secret.key").exists() {
            let _ = crate::fs_util::set_secret_permissions(&keys_dir.join("secret.key"));
        }
        if keys_dir.join("repo.key").exists() {
            let _ = crate::fs_util::set_secret_permissions(&keys_dir.join("repo.key"));
        }

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "backup": backup_file.to_string_lossy(),
                }))?
            );
        } else {
            println!("Restored from {}", backup_file.display());
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_crafted_tar_gz(archive_path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(archive_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            // Write raw path bytes into header directly so `../` paths can be tested too
            let name_bytes = name.as_bytes();
            let raw = header.as_gnu_mut().unwrap();
            raw.name[..name_bytes.len()].copy_from_slice(name_bytes);
            raw.name[name_bytes.len()..].fill(0);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn test_restore_rejects_entries_outside_shard_dir() {
        for bad_path in [
            "Makefile",
            "./Makefile",
            ".bashrc",
            ".shard.lock",
            "../evil",
        ] {
            let work = tempdir().unwrap();
            let tar_path = work.path().join("evil.tar.gz");
            write_crafted_tar_gz(
                &tar_path,
                &[
                    (".shard/HEAD", b"ref: refs/heads/main\n"),
                    (bad_path, b"pwned"),
                ],
            );

            let restore_dir = tempdir().unwrap();
            let res = restore(restore_dir.path(), &tar_path, true);
            assert!(
                res.is_err(),
                "expected restore() to reject archive entry '{}'",
                bad_path
            );
            assert!(
                !restore_dir.path().join("Makefile").exists(),
                "Makefile must never be extracted"
            );
            assert!(
                !restore_dir.path().join(".bashrc").exists(),
                ".bashrc must never be extracted"
            );
            assert!(
                !restore_dir.path().join(".shard").exists(),
                "partial .shard directory should be cleaned up on rejection"
            );
        }
    }

    #[test]
    fn test_backup_and_restore_roundtrip() {
        let src = tempdir().unwrap();
        crate::init(src.path(), "flat", "zstd", "fixed", None, false, true).unwrap();
        fs::write(src.path().join("weights.bin"), b"backup payload").unwrap();
        crate::add(src.path(), &src.path().join("weights.bin"), true).unwrap();
        let cid = crate::commit(src.path(), "backup commit", "Alice", true).unwrap();

        let archive_dir = tempdir().unwrap();
        let archive = archive_dir.path().join("repo.tar.gz");
        backup(src.path(), &archive, true).unwrap();

        let dst = tempdir().unwrap();
        restore(dst.path(), &archive, true).unwrap();
        crate::verify(dst.path(), &cid, true).unwrap();
        crate::checkout(dst.path(), &cid, true).unwrap();
        assert_eq!(
            fs::read(dst.path().join("weights.bin")).unwrap(),
            b"backup payload"
        );
    }
}
