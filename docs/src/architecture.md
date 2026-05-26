# Architecture

Shard is structured as a collection of decoupled crates, tied together by the main CLI entrypoint. 

## Component Diagram

```mermaid
graph TD
    CLI["shard CLI<br/>(clap argument parsing)"]
    
    subgraph Core ["core crate"]
        OpQueue["Operation Queue<br/>(read/write locking)"]
        Config["Config System<br/>(env overrides + validation)"]
        GC["Garbage Collector<br/>(DAG reachability scan)"]
        Tracing["Distributed Tracing<br/>(thread-local trace IDs)"]
        Metrics["Runtime Metrics<br/>(atomic counters)"]
        Chunker["Chunker<br/>(Fixed / Rabin)"]
        Compression["Compression<br/>(Zstd / Zlib)"]
        Store["Store<br/>(Sled / SQLite)"]
        CommitDAG["Commit DAG<br/>Manifest / Index / WAL<br/>Branch / Merge / Push"]
        API["HTTP API<br/>(axum /api/v1/*)"]
    end
    
    subgraph Net ["net crate"]
        LibP2P["libp2p Node<br/>TCP+Noise+Yamux<br/>mDNS / Kademlia<br/>Gossipsub<br/>Relay / DCUtR / AutoNAT<br/>Rate Limiting"]
    end
    
    subgraph Crypto ["crypto crate"]
        KeyGen["ed25519 key generation<br/>Signing & Verification<br/>Passphrase encryption<br/>Key rotation"]
    end
    
    subgraph Storage ["storage crate"]
        SledBackend["Sled Backend"]
        SQLiteBackend["SQLite Backend<br/>(r2d2 connection pool)"]
    end
    
    CLI --> Core
    CLI --> Net
    Core --> Crypto
    Core --> Storage
    Net --> Crypto
    Core -.-> OpQueue
    Core -.-> Config
    Core -.-> GC
    Core -.-> Tracing
    Core -.-> Metrics
    Core -.-> Chunker
    Core -.-> Compression
    Core -.-> Store
    Core -.-> CommitDAG
    Core -.-> API
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
| **Storage** | Sled, SQLite, or Flat file | Sled/SQLite for indexed queries; flat for portability |
| **P2P** | libp2p TCP+Noise+Yamux | Mature, NAT traversal via relay/DCUtR/AutoNAT |
| **Wire format** | JSON / CBOR | Serde over request-response + Gossipsub |
| **Concurrency** | Per-repo read-write queue | Reads parallel, writes exclusive; no global lock |
| **Config** | JSON + env var overrides | 12-factor friendly; `SHARD_*` env vars take precedence |
| **Tracing** | Thread-local trace IDs | Correlate logs across operations |
| **GC** | DAG reachability scan | Marks all reachable from HEAD/branches/tags/index, prunes rest |
| **Metrics** | Static atomic counters | Runtime operation counters with JSON snapshot |
