use serde::{Deserialize, Serialize};

fn default_compression() -> String {
    "none".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileManifest {
    pub name: String,
    pub size: u64,
    pub chunks: Vec<String>, // hex hashes
    pub content_type: Option<String>,
    /// Compression algorithm: "none", "zstd", "zlib"
    #[serde(default = "default_compression")]
    pub compression: String,
}
