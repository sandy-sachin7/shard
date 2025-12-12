# Shard

**Distributed, content-addressed version control for large ML artifacts — no cloud bills, no central bottlenecks.**

Shard is a protocol-first, local-first, peer-to-peer version control system designed specifically for machine learning artifacts (models, datasets, checkpoints). It runs entirely from developer machines and community hosts.

## Features

*   **Local-First:** No cloud dependency. Your data lives on your machine and your peers'.
*   **Content-Addressed:** Deduplication via Rabin fingerprinting (planned) and Blake3 hashing.
*   **P2P Distribution:** Fetch artifacts directly from peers using libp2p.
*   **Git-like CLI:** Familiar commands: `init`, `add`, `commit`, `pull`, `verify`.

## Installation

```bash
cargo install --path cmd/shard
```

## Usage

### Initialize a repository
```bash
shard init
```

### Add files
```bash
shard add <file>
```

### Commit changes
```bash
shard commit -m "Initial commit"
```

### Verify a commit
```bash
shard verify <commit_id>
### Share repository
```bash
shard share
# Output: Listening on /ip4/127.0.0.1/tcp/XXXXX
```

### Pull from peer
```bash
shard pull <multiaddr> <commit_id>
```

## Roadmap

*   [x] Phase 1: Local Core (Init, Add, Commit, Verify)
*   [x] Phase 2: Basic Network & Exchange (P2P, Pull, Share)
*   [ ] Phase 3: PubSub & Parallel Sync
*   [ ] Phase 4: Security & Provenance

## License

MIT
