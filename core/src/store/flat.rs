use anyhow::Result;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct FlatStore {
    root: PathBuf,
    lock: Mutex<()>,
}

impl FlatStore {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("objects.idx")
    }

    fn objects_dir(&self) -> PathBuf {
        self.root.join("objects")
    }

    fn append_index(&self, hash_hex: &str) -> Result<()> {
        let path = self.index_path();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(file, "{}", hash_hex)?;
        Ok(())
    }

    fn scan_and_index(&self) -> Result<Vec<(String, String)>> {
        let objects_dir = self.objects_dir();
        let mut entries = Vec::new();
        if objects_dir.exists() {
            for entry in fs::read_dir(&objects_dir)? {
                let entry = entry?;
                let prefix_dir = entry.path();
                let prefix_name = entry.file_name().to_string_lossy().to_string();
                if prefix_dir.is_dir() {
                    for file_entry in fs::read_dir(&prefix_dir)? {
                        let file_entry = file_entry?;
                        let hash = file_entry.file_name().to_string_lossy().to_string();
                        let rel_path = format!("{}/{}", prefix_name, hash);
                        entries.push((hash, rel_path));
                    }
                }
            }
        }
        let path = self.index_path();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        for (hash, _) in &entries {
            writeln!(file, "{}", hash)?;
        }
        Ok(entries)
    }

    pub fn put_chunk(&self, chunk: &crate::chunker::Chunk) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        let hash_hex = chunk.hash.to_hex().to_string();
        let prefix = hash_hex.get(..2).unwrap_or("xx");
        let dir = self.objects_dir().join(prefix);
        fs::create_dir_all(&dir)?;
        let path = dir.join(&hash_hex);
        if !path.exists() {
            fs::write(path, &chunk.data)?;
        }
        self.append_index(&hash_hex)?;
        Ok(())
    }

    pub fn get_chunk(&self, hash_hex: &str) -> Result<Vec<u8>> {
        if hash_hex.len() < 2 {
            anyhow::bail!("Invalid hash: {}", hash_hex);
        }
        let path = self.objects_dir().join(&hash_hex[..2]).join(hash_hex);
        if !path.exists() {
            anyhow::bail!("Chunk not found: {}", hash_hex);
        }
        Ok(fs::read(path)?)
    }

    pub fn has_chunk(&self, hash_hex: &str) -> bool {
        if hash_hex.len() < 2 {
            return false;
        }
        self.objects_dir()
            .join(&hash_hex[..2])
            .join(hash_hex)
            .exists()
    }

    pub fn iter_chunks(&self) -> Result<Vec<(String, String)>> {
        let _guard = self.lock.lock().unwrap();
        let idx_path = self.index_path();
        if idx_path.exists() {
            let file = fs::File::open(&idx_path)?;
            let mut entries = Vec::new();
            for line in std::io::BufReader::new(file).lines() {
                let h = line?.trim().to_string();
                if !h.is_empty() {
                    let prefix = h.get(..2).unwrap_or("xx");
                    entries.push((h.clone(), format!("{}/{}", prefix, h)));
                }
            }
            let file_count = count_object_files(&self.objects_dir());
            if entries.len() >= file_count {
                return Ok(entries);
            }
        }
        self.scan_and_index()
    }

    pub fn delete_chunk(&self, hash_hex: &str, full_path: Option<&str>) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        let path = if let Some(fp) = full_path {
            self.objects_dir().join(fp)
        } else {
            if hash_hex.len() < 2 {
                anyhow::bail!("Invalid hash: {}", hash_hex);
            }
            self.objects_dir().join(&hash_hex[..2]).join(hash_hex)
        };
        if path.exists() {
            fs::remove_file(path)?;
        }
        let idx_path = self.index_path();
        if idx_path.exists() {
            let entries: Vec<String> = {
                let file = fs::File::open(&idx_path)?;
                std::io::BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .map(|l| l.trim().to_string())
                    .filter(|h| !h.is_empty() && h != hash_hex)
                    .collect()
            };
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&idx_path)?;
            for h in &entries {
                writeln!(file, "{}", h)?;
            }
        }
        Ok(())
    }
}

fn count_object_files(objects_dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(dir) = fs::read_dir(objects_dir) {
        for entry in dir.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Ok(sub) = fs::read_dir(entry.path()) {
                    count += sub.count();
                }
            }
        }
    }
    count
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
        let result = store.get_chunk("abcdef");
        assert!(result.is_err());
    }

    #[test]
    fn test_flat_has_nonexistent() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        assert!(!store.has_chunk("ab"));
        assert!(!store.has_chunk(""));
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
    fn test_flat_put_idempotent() {
        let dir = tempdir().unwrap();
        let store = FlatStore::new(dir.path());
        let chunk = fake_chunk(b"idempotent");
        store.put_chunk(&chunk).unwrap();
        store.put_chunk(&chunk).unwrap();
        // Object file exists only once (deduped on disk)
        let hash_hex = chunk.hash.to_hex().to_string();
        let path = dir.path().join("objects").join(&hash_hex[..2]).join(&hash_hex);
        assert!(path.exists());
        // get_chunk works
        let retrieved = store.get_chunk(&hash_hex).unwrap();
        assert_eq!(retrieved, b"idempotent");
    }
}
