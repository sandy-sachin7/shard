use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShardRequest {
    GetManifest(String),
    GetChunk(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShardResponse {
    Manifest(Vec<u8>),
    Chunk(Vec<u8>),
    NotFound,
}
