# Shard

**Distributed, content-addressed version control for large ML artifacts — no cloud bills, no central bottlenecks.**

Shard is a protocol-first, local-first, peer-to-peer version control system designed specifically for machine learning artifacts (models, datasets, checkpoints). It runs entirely from developer machines and community hosts.

## Features

*   **Local-First:** No cloud dependency. Your data lives on your machine and your peers'.
*   **Content-Addressed:** Deduplication via fixed-size chunking and Blake3 hashing.
*   **P2P Distribution:** Fetch artifacts directly from peers using libp2p (TCP+Noise+Yamux).
*   **Git-like CLI:** Familiar commands: `init`, `add`, `commit`, `log`, `checkout`, `status`, `verify`, `prune`, `tag`, `config`, `pull`, `share`.
*   **Signed Commits:** Every commit is signed with ed25519.

## Installation

```bash
cargo install --path cmd/shard
```

## Usage

### Initialize a repository
```bash
shard init
# or as private (sets private=true in config):
shard init --private
```

### Add files
```bash
shard add <file>
```

### Commit changes
```bash
shard commit -m "Initial commit" --author "Alice"
```

### Show commit log
```bash
shard log
shard log --json   # JSON output
```

### Checkout files from a commit
```bash
shard checkout <commit_id>
shard checkout --json <commit_id>   # JSON output
```

### Show working tree status
```bash
shard status
shard status --json   # JSON output
```

### Verify a commit
```bash
shard verify <commit_id>
shard verify --json <commit_id>   # JSON output
```

### Prune unreachable objects
```bash
shard prune
```

### Manage tags
```bash
shard tag add <name> <commit_id>
shard tag list
```

### Manage config
```bash
shard config get              # list all
shard config get user.name    # get specific
shard config set user.name Alice
```

### Share repository
```bash
shard share
# Output: Listening on /ip4/0.0.0.0/tcp/XXXXX
```

### Pull from peer
```bash
shard pull <multiaddr> <commit_id>
```

### Add a peer
```bash
shard peer add /ip4/192.168.1.2/tcp/9876/p2p/<peer_id>
```

## Roadmap

*   [x] Phase 1: Local Core (Init, Add, Commit, Verify, Log, Checkout, Status, Config, Tag, Prune)
*   [x] Phase 2: Basic Network & Exchange (P2P, Pull, Share)
*   [ ] Phase 3: PubSub & Parallel Sync
*   [ ] Phase 4: Security & Provenance
*   [ ] Phase 5: UX & Packaging

## License

MIT
