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

/// Announcement payload for Gossipsub pubsub messages on the `shard:ann` topic.
/// Contains enough information for a peer to decide whether to fetch a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    /// The announced commit id (Blake3 hash of canonical commit JSON).
    pub commit_id: String,
    /// Number of files in the commit (from manifest(s)).
    pub file_count: u64,
    /// Total size of all files in bytes (sum of manifest sizes).
    pub total_size: u64,
    /// Human-readable repo name (from `shard config get repo_name` or `repo_id`).
    pub repo_name: String,
    /// Multiaddr string of the announcing peer (e.g., `/ip4/1.2.3.4/tcp/9000/p2p/<peer_id>`).
    pub peer_multiaddr: String,
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
    /// Peer denied our authentication.
    AuthDenied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_piece_roundtrip() {
        let piece = ChunkPiece {
            hash: "abc123".into(),
            offset: 42,
            size: 100,
            data: b"chunk data".to_vec(),
        };
        let json = serde_json::to_vec(&piece).unwrap();
        let piece2: ChunkPiece = serde_json::from_slice(&json).unwrap();
        assert_eq!(piece, piece2);
    }

    #[test]
    fn test_shard_request_roundtrip() {
        let cases = vec![
            ShardRequest::GetManifest("man1".into()),
            ShardRequest::GetChunk("chunk1".into()),
            ShardRequest::PutChunk(ChunkPiece {
                hash: "h".into(),
                offset: 0,
                size: 5,
                data: b"hello".to_vec(),
            }),
            ShardRequest::Authenticate {
                public_key: vec![1, 2, 3],
            },
            ShardRequest::AuthAnswer {
                signature: vec![4, 5, 6],
            },
        ];
        for req in cases {
            let json = serde_json::to_vec(&req).unwrap();
            let req2: ShardRequest = serde_json::from_slice(&json).unwrap();
            assert_eq!(req, req2);
        }
    }

    #[test]
    fn test_shard_response_roundtrip() {
        let cases = vec![
            ShardResponse::Manifest(b"manifest bytes".to_vec()),
            ShardResponse::Chunk(ChunkPiece {
                hash: "h".into(),
                offset: 99,
                size: 3,
                data: b"foo".to_vec(),
            }),
            ShardResponse::NotFound,
            ShardResponse::PutAck,
            ShardResponse::AuthChallenge {
                nonce: vec![7, 8, 9],
            },
            ShardResponse::AuthGranted,
            ShardResponse::AuthDenied,
        ];
        for resp in cases {
            let json = serde_json::to_vec(&resp).unwrap();
            let resp2: ShardResponse = serde_json::from_slice(&json).unwrap();
            assert_eq!(resp, resp2);
        }
    }

    #[test]
    fn test_chunk_piece_cbor_roundtrip() {
        let piece = ChunkPiece {
            hash: "cbor_hash".into(),
            offset: 123,
            size: 456,
            data: b"cbor data".to_vec(),
        };
        let mut out = Vec::new();
        ciborium::into_writer(&piece, &mut out).unwrap();
        let piece2: ChunkPiece = ciborium::from_reader(&out[..]).unwrap();
        assert_eq!(piece, piece2);
    }

    #[test]
    fn test_shard_request_cbor_roundtrip() {
        let req = ShardRequest::PutChunk(ChunkPiece {
            hash: "cbor_hash".into(),
            offset: 0,
            size: 1,
            data: vec![42],
        });
        let mut out = Vec::new();
        ciborium::into_writer(&req, &mut out).unwrap();
        let req2: ShardRequest = ciborium::from_reader(&out[..]).unwrap();
        assert_eq!(req, req2);
    }
}
