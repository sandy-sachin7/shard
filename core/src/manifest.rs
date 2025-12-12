use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileManifest {
    pub name: String,
    pub size: u64,
    pub chunks: Vec<String>, // hex hashes
    pub content_type: Option<String>,
}
