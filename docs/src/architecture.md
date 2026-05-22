# Architecture

Shard is structured as a collection of decoupled crates, tied together by the main CLI entrypoint. 

## Component Diagram

```mermaid
graph TD
    CLI["shard CLI<br/>(clap argument parsing)"]
    
    subgraph Core ["core crate"]
        Chunker["Chunker<br/>(Fixed / Rabin)"]
        Compression["Compression<br/>(Zstd / Zlib)"]
        Store["Store<br/>(Sled / SQLite)"]
        CommitDAG["Commit DAG<br/>Manifest / Index / WAL"]
    end
    
    subgraph Net ["net crate"]
        LibP2P["libp2p Node<br/>TCP+Noise+Yamux<br/>mDNS / Kademlia<br/>Gossipsub"]
    end
    
    subgraph Crypto ["crypto crate"]
        KeyGen["ed25519 key generation<br/>Signing & Verification"]
    end
    
    CLI --> Core
    CLI --> Net
    Core --> Crypto
    Net --> Crypto
    Core -.-> Chunker
    Core -.-> Compression
    Core -.-> Store
    Core -.-> CommitDAG
```

## Storage Layout

Shard stores all its data in a `.shard/` directory at the root of the repository.

```mermaid
graph LR
    Root[".shard/"] --> Objects["objects/"]
    Objects --> Prefix["<2-prefix>/"]
    Prefix --> Hash["<hash> (content-addressed chunks)"]
    
    Root --> HEAD["HEAD (current commit reference)"]
    Root --> Config["config.json (repository config)"]
    Root --> Index["index (staging area)"]
    Root --> Wal["wal.log (crash recovery)"]
    
    Root --> Keys["keys/"]
    Keys --> Sec["secret.key"]
    Keys --> Pub["public.key"]
    
    Root --> Refs["refs/heads/ (branch pointers)"]
    Root --> Auth["authorized_keys (P2P auth whitelist)"]
    Root --> Peers["peers.json (known P2P peers)"]
    Root --> Tags["tags.json (named commit pointers)"]
```

## Key Design Decisions

| Decision | Choice | Rationale |
| :--- | :--- | :--- |
| **Chunking** | Rabin (default) or Fixed | Rabin CDC improves dedup across versions; fixed for predictable sizes |
| **Compression** | Zstd or Zlib | Runtime selection; zstd is faster with better ratios |
| **Hashing** | Blake3 | Fastest cryptographic hash, SIMD-accelerated |
| **Signatures** | ed25519 | Proven, fast, small signatures (64 bytes) |
| **Storage** | Sled or Flat file | Sled embedded (zero deps); flat file for portability |
| **P2P** | libp2p TCP+Noise+Yamux | Mature, NAT traversal via relay/WebRTC planned |
| **Wire format** | JSON | Serde JSON over request-response |
