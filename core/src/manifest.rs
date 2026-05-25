use serde::{Deserialize, Serialize};

fn default_compression() -> String {
    "none".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileManifest {
    pub name: String,
    pub size: u64,
    pub chunks: Vec<String>,
    pub content_type: Option<String>,
    #[serde(default = "default_compression")]
    pub compression: String,
    #[serde(default)]
    pub merkle_root: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub signature: Option<String>,
}

impl FileManifest {
    pub fn merkle_root(chunk_hashes: &[String]) -> String {
        let mut hasher = blake3::Hasher::new();
        for h in chunk_hashes {
            hasher.update(h.as_bytes());
        }
        hasher.finalize().to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let m = FileManifest {
            name: "test.bin".into(),
            size: 1024,
            chunks: vec!["chunk1".into(), "chunk2".into()],
            content_type: Some("application/octet-stream".into()),
            compression: "zstd".into(),
            merkle_root: Some("merklehash".into()),
            created_by: Some("user".into()),
            created_at: Some(1000),
            signature: Some("sig".into()),
        };
        let json = serde_json::to_vec(&m).unwrap();
        let m2: FileManifest = serde_json::from_slice(&json).unwrap();
        assert_eq!(m2.name, "test.bin");
        assert_eq!(m2.size, 1024);
        assert_eq!(m2.chunks.len(), 2);
        assert_eq!(m2.compression, "zstd");
    }

    #[test]
    fn test_manifest_backward_compat() {
        let json = r#"{"name":"f.txt","size":50,"chunks":["h1"]}"#;
        let m: FileManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.compression, "none");
        assert!(m.content_type.is_none());
        assert!(m.merkle_root.is_none());
        assert!(m.created_by.is_none());
        assert!(m.signature.is_none());
    }

    #[test]
    fn test_manifest_default_compression() {
        let m = FileManifest {
            name: "n".into(),
            size: 0,
            chunks: vec![],
            content_type: None,
            compression: "none".into(),
            merkle_root: None,
            created_by: None,
            created_at: None,
            signature: None,
        };
        assert_eq!(m.compression, "none");
    }

    #[test]
    fn test_manifest_merkle_root_deterministic() {
        let chunks = vec!["a".into(), "b".into()];
        let root1 = FileManifest::merkle_root(&chunks);
        let root2 = FileManifest::merkle_root(&chunks);
        assert_eq!(root1, root2);
    }

    #[test]
    fn test_manifest_merkle_root_order_matters() {
        let ab = FileManifest::merkle_root(&["a".into(), "b".into()]);
        let ba = FileManifest::merkle_root(&["b".into(), "a".into()]);
        assert_ne!(ab, ba);
    }

    #[test]
    fn test_manifest_merkle_root_empty() {
        let root = FileManifest::merkle_root(&[]);
        assert!(!root.is_empty());
    }

    #[test]
    fn test_manifest_all_fields_json() {
        let m = FileManifest {
            name: "all.bin".into(),
            size: 999,
            chunks: vec!["c1".into(), "c2".into(), "c3".into()],
            content_type: Some("application/x-model".into()),
            compression: "zlib".into(),
            merkle_root: Some("mr".into()),
            created_by: Some("author".into()),
            created_at: Some(12345),
            signature: Some("signed".into()),
        };
        let json = serde_json::to_string_pretty(&m).unwrap();
        let m2: FileManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m2.content_type.unwrap(), "application/x-model");
        assert_eq!(m2.created_by.unwrap(), "author");
        assert_eq!(m2.created_at.unwrap(), 12345);
        assert_eq!(m2.signature.unwrap(), "signed");
    }
}
