# Introduction

**Distributed, content-addressed version control for large ML artifacts**

*No cloud bills, no central bottlenecks. Local-first, protocol-first, peer-to-peer.*

---

Shard is a protocol-first, local-first, peer-to-peer version control system for ML artifacts — models, datasets, checkpoints. It provides Git-like ergonomics, content-addressed storage, signed commits, and direct P2P transfers.

## Why Shard?

Machine Learning artifacts are often too large for traditional version control systems like Git, leading developers to rely on cloud storage solutions (S3, GCS) or Git LFS (which still requires centralized servers). 

Shard solves this by providing:
- **True Peer-to-Peer Distribution:** No central server required. Discover peers and sync artifacts directly.
- **Content-Addressed Storage:** Deduplication built-in through advanced chunking (Fixed or Rabin).
- **Git-like Ergonomics:** Familiar CLI commands (`add`, `commit`, `checkout`, `log`).
- **Cryptographic Provenance:** Every commit is signed using `ed25519` keys for verifiable history.

## Comparison

| Feature | Git | Shard |
| :--- | :--- | :--- |
| **Primary use** | Source code | ML artifacts (models, datasets, checkpoints) |
| **Chunking** | CDC (git fast-import) | Rabin + Fixed + configurable |
| **Compression** | zlib (default) | Zstd or Zlib (runtime selectable) |
| **Hashing** | SHA-1 / SHA-256 | Blake3 |
| **P2P** | Remote-centric | Native P2P (mDNS, Kademlia, Gossipsub) |
| **Storage backend** | Flat files + packfiles | Sled or SQLite indexed store |
| **Signing** | GPG (optional) | ed25519 (built-in, every commit) |
| **Transport** | SSH/HTTPS | libp2p TCP + Noise + Yamux |

## Performance

Shard is designed for large artifacts (100 MB – 100 GB). Key performance characteristics:

- **Chunking throughput**: ~1 GB/s (fixed), ~500 MB/s (Rabin CDC)
- **Compression**: Zstd level 3 — ~500 MB/s compress, ~2 GB/s decompress
- **Parallel pulls**: concurrent chunk requests — saturates available bandwidth
- **Memory**: bounded by configurable concurrency cap, not artifact size
