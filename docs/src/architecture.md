# Architecture

Shard is structured as a workspace of five decoupled crates plus native Python bindings (`shard-py`), orchestrated by `shard-core`.

## Component Diagram

```mermaid
graph TD
    CLI["shard-cli<br/>(clap CLI entrypoint)"]
    PY["shard-py<br/>(PyO3 native Python bindings)"]

    subgraph Core["shard-core"]
        OpQueue["Operation Queue + .shard.lock<br/>(in-process RW + OS fs2 lock)"]
        Config["Config System<br/>(config.json + SHARD_* env overrides)"]
        GC["Garbage Collector<br/>(DAG reachability + inbound pins)"]
        Tracing["Distributed Tracing<br/>(thread-local trace IDs)"]
        Metrics["Runtime Metrics<br/>(32 atomic counters)"]
        Chunker["Chunker<br/>(Fixed 4 MiB / Rabin CDC)"]
        Codec["SHRD v1 Chunk Codec<br/>(Zstd / Zlib / AES-256-GCM AAD)"]
        Store["Store Abstraction<br/>(FlatStore default / Sled / SQLite)"]
        CommitDAG["Commit DAG · Manifest · Index<br/>CRC32 WAL · Branch · Merge · HF"]
        API["HTTP API<br/>(Axum /api/v1/* + Bearer Auth)"]
    end

    subgraph Net["shard-net"]
        LibP2P["libp2p Node<br/>TCP + QUIC · Noise · Yamux<br/>mDNS · Kademlia · Gossipsub<br/>CBOR Req/Resp · 2-step Auth<br/>Circuit Relay v2 · DCUtR · AutoNAT"]
    end

    subgraph Crypto["shard-crypto"]
        KeyGen["Ed25519 KeyPair<br/>Argon2id + AES-256-GCM<br/>0o600 Atomic Persistence · Zeroize"]
    end

    subgraph Storage["shard-storage"]
        SledBackend["SledBackend<br/>(in-process handle cache)"]
        SQLiteBackend["SqliteBackend<br/>(WAL + r2d2 pool + truncate checkpoint)"]
    end

    CLI --> Core
    CLI --> Crypto
    PY --> Core
    Core --> Net
    Core --> Crypto
    Core --> Storage
```

## Storage Layout

Shard stores repository state inside `.shard/` and places the advisory OS lock file `.shard.lock` at the repository root (so even concurrent `shard init` calls are serialized).

```mermaid
graph LR
    Repo["&lt;repo_root&gt;/"] --> Lock[".shard.lock (OS fs2 advisory lock)"]
    Repo --> Root[".shard/"]

    Root --> Objects["objects/&lt;2-hex&gt;/&lt;64-hex&gt;<br/>(FlatStore, default)"]
    Root --> ObjectsDB["objects.db<br/>(Sled dir or SQLite WAL db)"]
    Root --> HEAD["HEAD (branch ref or commit ID)"]
    Root --> Config["config.json (repository config)"]
    Root --> Index["index (staging area)"]
    Root --> Wal["wal.log (CRC32 crash recovery WAL)"]
    Root --> Partial["partial/ (resumable P2P transfers)"]

    Root --> Keys["keys/"]
    Keys --> Sec["secret.key (0o600, optional Argon2id+AES-GCM)"]
    Keys --> Pub["public.key"]
    Keys --> RepoKey["repo.key (private AES-256-GCM key)"]
    Keys --> Current["current (active key_id)"]
    Keys --> Records["records/ &amp; rotations/ &amp; archive/"]

    Root --> Refs["refs/heads/ (branch pointers)"]
    Root --> Auth["authorized_keys (P2P auth allowlist)"]
    Root --> Peers["peers.json (trusted P2P peer multiaddrs)"]
    Root --> Tags["tags.json (named commit tags)"]
```

## Key Design Decisions

| Decision | Choice | Rationale |
| :--- | :--- | :--- |
| **Chunking** | Fixed 4 MiB (default) or Rabin CDC (`--chunker rabin`) | Fixed for predictable I/O and peak throughput; Rabin CDC for byte-shift-resilient checkpoint dedup |
| **Compression & Encryption** | `SHRD` v1 self-describing 8-byte envelope (Zstd/Zlib/None + AES-256-GCM AAD) | Safe dedup across compression changes; AAD binds ciphertext to `chunk_id` |
| **Hashing** | Blake3 | SIMD-accelerated 256-bit cryptographic hash for content addressing and Merkle roots |
| **Signatures** | Ed25519 | Fast, compact 64-byte signatures with signed key rotation and trust-anchor verification |
| **Storage** | `FlatStore` (default), `Sled`, or `SQLite` | `FlatStore` for simple zero-dependency portability; `Sled`/`SQLite` for indexed single-DB storage |
| **P2P** | libp2p TCP + QUIC + Noise + Yamux | Mature encrypted transports with Circuit Relay v2, DCUtR, and AutoNAT |
| **Wire Format** | CBOR (request-response) + JSON (Gossipsub) | Compact binary framing for object transfer; inspectable pubsub announcements |
| **Concurrency** | In-process `RwLock` + `.shard.lock` OS advisory lock | Readers run in parallel, writers are exclusive across threads and processes |
| **Config** | JSON + `SHARD_*` env var overrides | 12-factor friendly with schema validation before repository mutation |
| **Tracing & Metrics** | Thread-local trace IDs + 32 static atomic counters | Correlate diagnostic logs on `stderr` and export health snapshots via `--json` |
| **GC** | DAG reachability + inbound pin scan | Marks reachable objects from HEAD, branches, tags, index, and active P2P pushes before pruning |
