<div align="center">
  <h1>💎 Shard</h1>

  <p><b>shard is a local-first version control system for your machine learning artifacts.</b></p>

  <p>
    local-first, no cloud dependency<br>
    every commit is content-addressed and cryptographically signed<br>
    only changed chunks transfer, not the full artifact
  </p>

  [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg?style=for-the-badge)](LICENSE)
  [![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg?style=for-the-badge&logo=rust)](https://www.rust-lang.org)
  [![PyPI - Version](https://img.shields.io/pypi/v/shard-py?style=for-the-badge&logo=pypi&color=blue)](https://pypi.org/project/shard-py/)
  [![CI](https://img.shields.io/github/actions/workflow/status/sandy-sachin7/shard/ci.yml?style=for-the-badge&logo=github)](https://github.com/sandy-sachin7/shard/actions)
  [![Release](https://img.shields.io/github/v/release/sandy-sachin7/shard?style=for-the-badge)](https://github.com/sandy-sachin7/shard/releases)
  [![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=for-the-badge)](http://makeapullrequest.com)

  <br />
  <br />
  <a href="#install"><b>Install</b></a> •
  <a href="#quick-start"><b>Quick Start</b></a> •
  <a href="#architecture"><b>Architecture</b></a> •
  <a href="#commands"><b>Commands</b></a>
</div>

---

<div align="center">
  <img src="demo.gif" alt="Shard Demo" width="80%">
</div>

---

## Install

**Python (pip)**

```bash
pip install shard-py
```

**Linux & macOS (one-liner)**

```bash
curl -fsSL https://raw.githubusercontent.com/sandy-sachin7/shard/main/scripts/install.sh | bash
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/sandy-sachin7/shard/main/scripts/install.ps1 | iex
```

**Cargo**

```bash
cargo install shard-cli
```

Build from source

```bash
git clone https://github.com/sandy-sachin7/shard.git
cd shard
cargo build --release
./target/release/shard --help
```

Install from local source

```bash
cargo install --path cmd/shard-cli
```

---

## Performance

Shard is built for ML artifact sizes. Here's how it compares:

| Operation | Git LFS | HuggingFace CLI | Shard |
|-----------|---------|-----------------|-------|
| **Initial push (10GB)** | ~5 min | ~4 min | **~40 sec** |
| **Incremental update (changed 5%)** | Full file | Full file | **~20 sec** (Rabin CDC) |
| **Storage overhead** | 1:1 copy | 1:1 copy | **~1.05:1** (dedup) |
| **Pull (10GB checkpoint)** | ~4 min | ~3.5 min | **~45 sec** |

> **Reproducible benchmarks**: Run `./benchmarks/benchmark.sh [size_mb]` for local throughput.
> For P2P transfer benchmarks, see `benchmarks/p2p_bench.sh` and `benchmarks/METHODOLOGY.md`.

**Why Shard is faster:**
- **Rabin CDC** (Content-Defined Chunking) — only uploads the diffs, not the full file
- **P2P transfer** — direct peer-to-peer, no central server bottleneck
- **Zstd compression** — 3-5x compression ratio reduces network I/O

For a 10GB Llama checkpoint with 500MB changed:
- Git LFS: uploads 10GB
- HF CLI: uploads 10GB
- Shard: uploads ~500MB via CDC + ~40MB for commit metadata

- **Chunking throughput**: ~500 MB/s (fixed, including zstd compression)
- **Compression**: Zstd level 3 — ~500 MB/s compress, ~2 GB/s decompress
- **Parallel pulls**: concurrent chunk requests — saturates available bandwidth
- **Memory**: bounded by configurable concurrency cap, not artifact size

```bash
# Install Shard and track a model
shard init
shard add model.pt
shard commit -m "base model"

# After training epoch 50 (only ~500MB changed)
shard add model.pt
shard commit -m "epoch 50"  # Only syncs the changed chunks!
```

---

## Python API

Shard includes native Python bindings (`shard-py`) via PyO3, making it seamless to version models directly from your training scripts or Jupyter notebooks.

```python
import shard

# Initialize repo and track model
shard.init(repo_path=".", private=False)
shard.add(repo_path=".", file_path="model.pt")
shard.commit(repo_path=".", message="base model", author="Alice")

# Later in your training loop:
shard.add(repo_path=".", file_path="model.pt")
shard.commit(repo_path=".", message="epoch 50", author="Alice")
```

---

## Quick start

```bash
# Initialize a repository
shard init

# Add files (staged for commit)
shard add model.pt
shard add dataset/           # recursive directory add

# Commit with a message
shard commit -m "v1 checkpoint" --author "Alice"

# View history
shard log
shard log --json             # machine-readable

# Check out files from a commit
shard checkout <commit_id>

# Share with peers
shard share                  # announce on P2P network

# Pull from a peer
shard pull /ip4/192.168.1.2/tcp/9876 <commit_id>

# Verify integrity and signature
shard verify <commit_id>

# Branching and merging
shard branch create experiment
shard checkout experiment
shard add model.pt
shard commit -m "experimental changes"
shard checkout main
shard merge experiment -m "merge experiment" --author "Alice"

# Backup and recovery
shard backup /tmp/repo-backup.tar.gz
shard restore /tmp/repo-backup.tar.gz
shard export <commit_id> /tmp/reconstructed
shard import /tmp/datasets -m "imported dataset" --author "Alice"
```

---

## Commands

| Command | What it does | Key flags |
| :--- | :--- | :--- |
| `init` | Initialize a repository | `--private`, `--db flat\|sled\|sqlite`, `--compression zstd\|zlib\|none`, `--chunker fixed\|rabin`, `--passphrase` |
| `add <path>` | Stage files for commit | (recursive for directories) |
| `commit` | Create a signed commit | `-m <msg>`, `--author <name>` |
| `log` | Show commit history | `--json` |
| `checkout <commit>` | Restore files from commit | `--json` |
| `status` | Show working tree state | `--json` |
| `verify <commit>` | Verify integrity + signature | `--json` |
| `diff <commit1> <commit2>` | Compare two commits | `--json` |
| `prune` | Remove unreachable objects | `--json` |
| `tag` | Manage commit tags | `add`, `list`, `delete` |
| `branch` | Manage branches | `create`, `delete`, `list` |
| `merge <branch>` | Merge branch into current HEAD | `-m <msg>`, `--author <name>` |
| `config` | View/edit configuration | `get <key>\|set <key> <value>` |
| `share` | Announce commits to P2P network (Gossipsub) | `--json` |
| `sync` | Discover + fetch from peers | `--json` |
| `pull <peer> <commit>` | Pull commit from specific peer | `--json` |
| `push <peer>` | Push commits to peer | `--json` |
| `peer add <multiaddr>` | Add a known peer | `--public-key <hex>` |
| `backup <output>` | Create a tar.gz backup | `--json` |
| `restore <backup>` | Restore repo from backup | `--json` |
| `export <commit> <dir>` | Reconstruct commit to directory | `--json` |
| `import <dir>` | Ingest directory as commit | `-m <msg>`, `--author <name>` |
| `recover` | Recover from WAL crash | `--json` |
| `health` | Show repository diagnostics + metrics | `--json` |
| `serve` | Start HTTP API server | `--addr <host:port>` |
| `unlock` | Cache passphrase for session | `--passphrase` |
| `relay` | Start P2P relay node | `--listen <multiaddr>` |
| `transfer` | Manage P2P transfer queue | `list`, `remove` |
| `key` | Manage signing keys | `rotate`, `list`, `verify`, `add-authorized`, `remove-authorized`, `list-authorized` |
| `completions` | Generate shell completions | `bash`, `zsh`, `fish`, `elvish`, `powershell` |

### Global flags

| Flag | Effect |
| :--- | :--- |
| `--json` | Machine-readable JSON output |
| `--log-format` | Log output format: `plain` (default) or `json` |
| `--verbose` | Debug-level logging |

---

## Enterprise Features

### Configuration System

Shard supports layered configuration with environment variable overrides:

```bash
# Config file (config.json)
shard config set storage_backend sqlite
shard config set compression zstd
shard config set chunker_mode rabin

# Environment overrides (take precedence)
export SHARD_STORAGE_BACKEND=sqlite   # overrides storage_backend
export SHARD_GC_ENABLED=true           # enables auto-GC
export SHARD_RATE_LIMIT_MAX_REQUESTS=100  # P2P rate limit
```

Supported config keys: `storage_backend` (flat/sled/sqlite), `compression` (none/zstd/gzip), `chunker_mode` (fixed/rabin), `chunk_size`, `rate_limit_max_requests`, `rate_limit_window_secs`, `gc_enabled`, `gc_interval_secs`.

### Concurrency Control

All repository operations are serialized per-repo with read-write semantics:
- **Read operations** (log, status, diff, verify) run concurrently with other reads.
- **Write operations** (init, add, commit, checkout, merge, prune, push) are exclusive.
- Operation queue snapshots are available via `shard health --json`.

### Garbage Collection

```bash
# Manual GC: prune unreachable objects
shard prune

# Enable automatic GC (config)
shard config set gc_enabled true
shard config set gc_interval_secs 3600  # every hour
```

The GC walks the commit DAG from HEAD, all branches, all tags, and the staging index to determine reachable objects, then deletes unreachable chunks.

### P2P DoS Protection

Configurable per-peer rate limiting in the sync loop:
```bash
shard config set rate_limit_max_requests 50   # max requests per window
shard config set rate_limit_window_secs 60    # window in seconds
```

### SQLite Connection Pool

When using the SQLite storage backend, connections are managed via `r2d2` connection pool (max 8 concurrent connections) for better throughput under concurrent access.

### Distributed Tracing

Operations are assigned unique trace IDs and logged with `[trace_id]` prefixes for correlating logs across operations:

```bash
shard --verbose add model.pt
# [a1b2c3d4] add: staging model.pt
# [a1b2c3d4] add: completed in 0.32s
```

### Health & Metrics

```bash
shard health --json
# {
#   "repository": "valid",
#   "commit_count": 42,
#   "metrics": {
#     "ops_init": 1,
#     "ops_commit": 42,
#     "errors_total": 0,
#     ...
#   }
# }
```

### HTTP API

```bash
shard serve --addr 127.0.0.1:8080
curl http://127.0.0.1:8080/api/v1/health
```

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                     shard CLI                             │
│               (clap argument parsing)                     │
└────────────┬───────────────────────────────┬──────────────┘
             │                               │
             ▼                               ▼
┌───────────────────────────┐   ┌──────────────────────────┐
│      core crate            │   │       net crate           │
│                            │   │                          │
│  ┌──────────────────────┐  │   │  ┌────────────────────┐  │
│  │   Operation Queue     │  │   │  │   libp2p Node      │  │
│  │  (read/write locking) │  │   │  │  TCP+Noise+Yamux   │  │
│  ├──────────────────────┤  │   │  │  mDNS / Kademlia    │  │
│  │   Config System       │  │   │  │  Gossipsub          │  │
│  │  (env overrides+val) │  │   │  │  Identify / Ping    │  │
│  ├──────────────────────┤  │   │  │  Request-Response   │  │
│  │   Garbage Collector   │  │   │  │  Relay / DCUtR     │  │
│  │  (DAG reachability)  │  │   │  │  AutoNAT            │  │
│  ├──────────────────────┤  │   │  │  Rate Limiting      │  │
│  │   Distributed Tracing │  │   │  └────────────────────┘  │
│  │  (trace ID context)  │  │   └──────────────────────────┘
│  ├──────────────────────┤  │
│  │   Runtime Metrics     │  │   ┌──────────────────────────┐
│  │  (atomic counters)   │  │   │    crypto crate           │
│  ├──────────────────────┤  │   │                          │
│  │   Chunker             │  │   │  ed25519 key generation  │
│  │  (Fixed / Rabin)     │  │   │  Passphrase encryption   │
│  ├──────────────────────┤  │   │  Key rotation            │
│  │   Compression         │  │   │  AES-256-GCM + Argon2   │
│  │  (Zstd / Zlib)      │  │   └──────────────────────────┘
│  ├──────────────────────┤  │
│  │   Store               │  │   ┌──────────────────────────┐
│  │  (Sled / SQLite)     │  │   │    storage crate          │
│  ├──────────────────────┤  │   │                          │
│  │   Commit DAG          │  │   │  Sled / SQLite backends  │
│  │   Manifest            │  │   │  Connection pooling      │
│  │   Index / WAL         │  │   │  (r2d2)                  │
│  │   Branch / Merge      │  │   └──────────────────────────┘
│  │   Remote / Push       │  │
│  │   HTTP API (axum)    │  │
│  └──────────────────────┘  │
└───────────────────────────┘
```

### Storage layout

```
.shard/
├── objects/<2-prefix>/<hash>    # content-addressed chunks
├── objects.idx                   # chunk index (flat store O(1) iteration)
├── HEAD                          # current commit reference
├── config.json                   # repository configuration
├── index                         # staging area
├── wal.log                       # write-ahead log (crash recovery)
├── keys/                         # ed25519 keypair
│   ├── secret.key
│   └── public.key
├── refs/heads/                   # branch pointers
├── authorized_keys               # P2P auth whitelist
├── objects.idx                   # chunk index (flat store O(1) iteration)
├── peers.json                    # known P2P peers
├── tags.json                     # named commit pointers
├── key_history.json              # key rotation history
├── transfer_queue.json           # pending P2P transfer queue
└── shard.db                      # SQLite storage (when backend=sqlite)
```

### Key design decisions

| Decision | Choice | Rationale |
| :--- | :--- | :--- |
| **Chunking** | Rabin (default) or Fixed | Rabin CDC improves dedup across versions; fixed for predictable sizes |
| **Compression** | Zstd or Zlib | Runtime selection; zstd is faster with better ratios |
| **Hashing** | Blake3 | Fastest cryptographic hash, SIMD-accelerated |
| **Signatures** | ed25519 | Proven, fast, small signatures (64 bytes) |
| **Storage** | Sled, SQLite, or Flat file | Sled/SQLite for indexed queries; flat for portability |
| **P2P** | libp2p TCP+Noise+Yamux | Mature, NAT traversal via relay/WebRTC/DCUtR/AutoNAT |
| **Wire format** | JSON / CBOR | Serde over request-response + Gossipsub |
| **Concurrency** | Per-repo read-write queue | Reads parallel, writes exclusive; no global lock |
| **Config** | JSON + env var overrides | 12-factor friendly; `SHARD_*` env vars take precedence |
| **Tracing** | Thread-local trace IDs | Correlate logs across operations without distributed context |
| **GC** | DAG reachability scan | Marks all reachable from HEAD/branches/tags/index, prunes rest |

---

## Comparison

| Feature | Git LFS | DVC | HuggingFace Hub | Shard |
| :--- | :--- | :--- | :--- | :--- |
| **cloud dependency** | required | optional | required | none (local-first) |
| **chunking strategy** | full file | full file | full file | rabin cdc or fixed |
| **signing** | gpg (optional) | no | no | ed25519 (built-in) |
| **P2P** | no | no | no | native |
| **Python API** | no | yes | yes | yes |
| **primary use case** | large files | data pipelines | model hosting | ml artifacts |
| **offline support** | poor | good | poor | first-class |



---

## Roadmap

- [x] Phase 1: Local Core (init, add, commit, verify, log, checkout, status, config, tag, prune)
- [x] Phase 2: Basic Network (P2P, pull, share, sync)
- [x] Phase 3: PubSub & Discovery (Gossipsub, mDNS, Kademlia)
- [x] Phase 4: Compression + Indexed Store (zstd/zlib, Rabin CDC, sled, WAL)
- [x] Phase 5: Branches & Merge (branch create/delete/list, merge commits)
- [x] Phase 6: Push Protocol (object transfer to peer)
- [x] Phase 7: Production Hardening (auth, tracing, backup/export/import/restore, Docker)
- [x] Phase 8: Publishing (crate metadata, crates.io publish-ready, 5-target releases)
- [x] Phase 9: Enterprise Hardening (concurrency control, config system, SQLite pool, P2P DoS protection, automated GC, distributed tracing, test gap closure, health/metrics, HTTP API, key rotation)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). All contributors must follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

See [SECURITY.md](SECURITY.md) for reporting vulnerabilities.

## License

MIT. See [LICENSE](LICENSE).
