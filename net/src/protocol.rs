use serde::{Deserialize, Serialize};

/// A chunk transferred over the wire includes its content hash, file offset,
/// data size, and raw bytes — enabling per-chunk verification before store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkPiece {
    /// Blake3 hex hash of the uncompressed, unencrypted original data.
    pub hash: String,
    /// Byte offset within the original file (0 if unknown at serving time).
    pub offset: u64,
    /// Number of raw bytes in `data`.
    pub size: u64,
    /// The raw chunk bytes (compressed & encrypted for private repos).
    pub data: Vec<u8>,
}

/// Request messages for Shard's request-response P2P protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShardRequest {
    /// Request the raw bytes of a manifest object by its content hash.
    GetManifest(String),
    /// Request the raw bytes of a chunk object by its content hash.
    GetChunk(String),
    /// Push a chunk to the peer with piece headers {hash, offset, size, data}.
    PutChunk(ChunkPiece),
    /// Initiate P2P authentication: send our public key to the peer.
    Authenticate { public_key: Vec<u8> },
    /// Respond to an auth challenge with our signature over the nonce.
    AuthAnswer { signature: Vec<u8> },
}

/// Response messages for Shard's request-response P2P protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShardResponse {
    /// The raw bytes of the requested manifest.
    Manifest(Vec<u8>),
    /// A chunk with piece headers {hash, offset, size, data}.
    Chunk(ChunkPiece),
    /// The requested object was not found in the peer's store.
    NotFound,
    /// Acknowledgement that a `PutChunk` was accepted.
    PutAck,
    /// Challenge nonce from the peer during P2P auth handshake.
    AuthChallenge { nonce: Vec<u8> },
    /// Peer accepted our authentication.
    AuthGranted,
    /// Peer rejected our authentication.
    AuthDenied,
}
