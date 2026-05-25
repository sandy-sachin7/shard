use crate::manifest::FileManifest;
use crate::metadata::{self, MetadataFormat};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Index {
    pub files: HashMap<String, FileManifest>,
}

impl Index {
    pub fn load(path: &Path, _fmt: &MetadataFormat) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read(path)?;
        metadata::deserialize(&content)
    }

    pub fn save(&self, path: &Path, fmt: &MetadataFormat) -> Result<()> {
        let content = metadata::serialize(self, fmt);
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_manifest(name: &str) -> FileManifest {
        FileManifest {
            name: name.to_string(),
            size: 100,
            chunks: vec!["hash1".into()],
            content_type: None,
            compression: "none".into(),
            merkle_root: None,
            created_by: None,
            created_at: None,
            signature: None,
        }
    }

    #[test]
    fn test_index_load_nonexistent() {
        let dir = tempdir().unwrap();
        let idx = Index::load(&dir.path().join("index"), &MetadataFormat::Json).unwrap();
        assert!(idx.files.is_empty());
    }

    #[test]
    fn test_index_save_load_json_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index");

        let mut idx = Index::default();
        idx.files.insert("a.txt".into(), make_manifest("a.txt"));
        idx.files.insert("b.txt".into(), make_manifest("b.txt"));
        idx.save(&path, &MetadataFormat::Json).unwrap();

        let loaded = Index::load(&path, &MetadataFormat::Json).unwrap();
        assert_eq!(loaded.files.len(), 2);
        assert!(loaded.files.contains_key("a.txt"));
        assert!(loaded.files.contains_key("b.txt"));
    }

    #[test]
    fn test_index_save_load_cbor_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index");

        let mut idx = Index::default();
        idx.files.insert("c.txt".into(), make_manifest("c.txt"));
        idx.save(&path, &MetadataFormat::Cbor).unwrap();

        let loaded = Index::load(&path, &MetadataFormat::Cbor).unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert!(loaded.files.contains_key("c.txt"));

        // CBOR file should auto-detect format
        let loaded_auto = Index::load(&path, &MetadataFormat::Json).unwrap();
        assert_eq!(loaded_auto.files.len(), 1);
    }

    #[test]
    fn test_index_overwrite() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index");

        let mut idx = Index::default();
        idx.files.insert("x.txt".into(), make_manifest("x.txt"));
        idx.save(&path, &MetadataFormat::Json).unwrap();

        let mut idx2 = Index::default();
        idx2.files.insert("y.txt".into(), make_manifest("y.txt"));
        idx2.save(&path, &MetadataFormat::Json).unwrap();

        let loaded = Index::load(&path, &MetadataFormat::Json).unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert!(loaded.files.contains_key("y.txt"));
    }
}
