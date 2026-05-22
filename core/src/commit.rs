use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Commit {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub manifests: Vec<String>,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub public_key: Option<String>,
    pub signature: Option<String>,
}
