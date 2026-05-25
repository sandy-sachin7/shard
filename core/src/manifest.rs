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
