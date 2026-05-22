use serde::{Deserialize, Serialize};

/// Request messages for Shard's request-response P2P protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShardRequest {
    /// Request the raw bytes of a manifest object by its content hash.
    GetManifest(String),
    /// Request the raw bytes of a chunk object by its content hash.
    GetChunk(String),
    /// Push a raw chunk object to the peer. The `id` is the Blake3 hash of `data`.
    PutChunk { id: String, data: Vec<u8> },
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
    /// The raw bytes of the requested chunk.
    Chunk(Vec<u8>),
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
