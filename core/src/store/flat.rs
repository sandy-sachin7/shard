use crate::chunker::Chunk;
use crate::fs_util::{atomic_write, fsync_dir, validate_object_id};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Flat-file content-addressed object store (`objects/<2-hex>/<64-hex>`).
/// Writes are crash-safe (temp file + `sync_all()` + atomic rename) and all object
/// IDs are strictly validated as 64 lowercase hex characters.
pub struct FlatStore {
    root: PathBuf,
}

impl FlatStore {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    fn objects_dir(&self) -> PathBuf {
        self.root.join("objects")
    }

    fn object_path(&self, hash_hex: &str) -> Result<PathBuf> {
        validate_object_id(hash_hex)?;
        Ok(self.objects_dir().join(&hash_hex[..2]).join(hash_hex))
    }

    pub fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        let hash_hex = chunk.hash.to_hex().to_string();
        let path = self.object_path(&hash_hex)?;
        if !path.exists() {
            atomic_write(&path, &chunk.data)?;
        }
        Ok(())
    }

    pub fn put_chunk_overwrite(&self, chunk: &Chunk) -> Result<()> {
        let hash_hex = chunk.hash.to_hex().to_string();
        let path = self.object_path(&hash_hex)?;
        atomic_write(&path, &chunk.data)?;
        Ok(())
    }

    pub fn get_chunk(&self, hash_hex: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash_hex)?;
        if !path.exists() {
            anyhow::bail!("Chunk not found: {}", hash_hex);
        }
        Ok(fs::read(path)?)
    }

    pub fn has_chunk(&self, hash_hex: &str) -> bool {
        self.object_path(hash_hex)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Authoritative directory walk over `objects/<00..ff>/` returning only valid
    /// 64-character lowercase hex object IDs (ignoring any stray temp files).
    pub fn iter_chunks(&self) -> Result<Vec<(String, String)>> {
        let objects_dir = self.objects_dir();
        let mut entries = Vec::new();
        if !objects_dir.exists() {
            return Ok(entries);
        }
        for entry in fs::read_dir(&objects_dir)? {
            let entry = entry?;
            let prefix_dir = entry.path();
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let prefix_name = entry.file_name().to_string_lossy().to_string();
            if prefix_name.len() != 2
                || !prefix_name
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                continue;
            }
            for file_entry in fs::read_dir(&prefix_dir)? {
                let file_entry = file_entry?;
                if !file_entry.file_type()?.is_file() {
                    continue;
                }
                let hash = file_entry.file_name().to_string_lossy().to_string();
                if validate_object_id(&hash).is_ok() && hash.starts_with(&prefix_name) {
                    let rel_path = format!("{}/{}", prefix_name, hash);
                    entries.push((hash, rel_path));
                }
            }
        }
        Ok(entries)
    }

    /// Delete a validated object in O(1) time. Ignores unvalidated `full_path` overrides
    /// and always derives the safe path directly from the validated `hash_hex`.
    pub fn delete_chunk(&self, hash_hex: &str, _full_path: Option<&str>) -> Result<()> {
        let path = self.object_path(hash_hex)?;
        if path.exists() {
            fs::remove_file(&path)?;
            if let Some(parent) = path.parent() {
                fsync_dir(parent);
            }
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        let dir = self.objects_dir();
        if dir.exists() {
            fsync_dir(&dir);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::Chunk;
    use tempfile::tempdir;

    fn fake_chunk(data: &[u8]) -> Chunk {
        let hash = blake3::hash(data);
        Chunk {
            hash,
            data: data.to_vec(),
            offset: 0,
        }
    }

    #[test]
    fn test_flat_put_get_roundtrip() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        let chunk = fake_chunk(b"hello flat store");
        store.put_chunk(&chunk).unwrap();
        let hash_hex = chunk.hash.to_hex().to_string();
        assert!(store.has_chunk(&hash_hex));
        let retrieved = store.get_chunk(&hash_hex).unwrap();
        assert_eq!(retrieved, b"hello flat store");
    }

    #[test]
    fn test_flat_get_nonexistent() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        assert!(store.get_chunk(&"0".repeat(64)).is_err());
        assert!(store.get_chunk("abcdef").is_err());
        assert!(store.get_chunk("/etc/passwd").is_err());
        assert!(store.get_chunk("../../../keys/secret.key").is_err());
        assert!(store.get_chunk("éé").is_err());
    }

    #[test]
    fn test_flat_has_nonexistent() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        assert!(!store.has_chunk("ab"));
        assert!(!store.has_chunk(""));
        assert!(!store.has_chunk("éé"));
        assert!(!store.has_chunk("/etc/passwd"));
    }

    #[test]
    fn test_flat_delete_chunk() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        let chunk = fake_chunk(b"delete me");
        store.put_chunk(&chunk).unwrap();
        let hash_hex = chunk.hash.to_hex().to_string();
        assert!(store.has_chunk(&hash_hex));
        store.delete_chunk(&hash_hex, None).unwrap();
        assert!(!store.has_chunk(&hash_hex));
    }

    #[test]
    fn test_flat_iter_chunks() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        let chunks = vec![
            fake_chunk(b"chunk a"),
            fake_chunk(b"chunk b"),
            fake_chunk(b"chunk c"),
        ];
        for c in &chunks {
            store.put_chunk(c).unwrap();
        }
        let entries = store.iter_chunks().unwrap();
        assert_eq!(entries.len(), 3);
        for c in &chunks {
            let h = c.hash.to_hex().to_string();
            assert!(entries.iter().any(|(hash, _)| hash == &h));
        }
    }

    #[test]
    fn test_flat_put_idempotent_no_duplicates_in_iter() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        let chunk = fake_chunk(b"idempotent");
        store.put_chunk(&chunk).unwrap();
        store.put_chunk(&chunk).unwrap();
        let hash_hex = chunk.hash.to_hex().to_string();
        let entries = store.iter_chunks().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(store.get_chunk(&hash_hex).unwrap(), b"idempotent");
    }
}
