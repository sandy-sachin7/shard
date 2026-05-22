# Shard Protocol

## Version 1.0.0-draft

### Transport

- TCP with Noise handshake (ed25519) and Yamux multiplexing
- Stream protocol ID: `/shard/1`
- CBOR-encoded messages over libp2p request-response

### Message Types

```rust
enum ShardRequest {
    GetManifest(String),       // manifest hash
    GetChunk(String),          // chunk hash
    PushManifest(String, Vec<u8>),  // hash + data
    Announce(String),          // commit_id
}

enum ShardResponse {
    Manifest(Vec<u8>),
    Chunk(Vec<u8>),
    NotFound,
    Ack,
}
```

### Discovery

- **mDNS**: LAN peer discovery
- **Kademlia DHT**: distributed hash table for peer routing
- **Gossipsub**: pubsub for commit announcements on topic `/shard/repo/<repo_id>`
- **Identify**: protocol version and listen address exchange

### Pull Flow (current — 3 round-trips)

1. Request commit by hash → response
2. Request manifests by hash (parallel) → response
3. Request chunks by hash (parallel) → response

### Storage Layout

```
.shard/
├── objects/
│   └── <2-hex-prefix>/
│       └── <full-hex-hash>    # raw chunk data
├── HEAD                        # current commit ref
├── config.json                 # repo configuration
├── index                       # staging area (JSON)
├── keys/
│   ├── secret.key              # ed25519 signing key
│   └── public.key              # ed25519 verification key
├── peers.json                  # known peer multiaddrs
└── tags.json                   # tag -> commit mapping
```
