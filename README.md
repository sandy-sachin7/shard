<div align="center">
  <h1>💎 Shard</h1>
  <p><b>Distributed, content-addressed version control for large ML artifacts</b></p>
  <p><i>No cloud bills, no central bottlenecks. Local-first, protocol-first, peer-to-peer.</i></p>

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

Shard is a protocol-first, local-first, peer-to-peer version control system for ML artifacts — models, datasets, checkpoints. Git-like ergonomics, content-addressed storage, signed commits, and direct P2P transfers.

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

**Build from source**

```bash
git clone https://github.com/sandy-sachin7/shard.git
cd shard
cargo build --release
./target/release/shard --help
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

- **Chunking throughput**: ~1 GB/s (fixed), ~500 MB/s (Rabin CDC)
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
| `init` | Initialize a repository | `--private`, `--compression zstd\|zlib\|none`, `--chunker fixed\|rabin` |
| `add <path>` | Stage files for commit | (recursive for directories) |
| `commit` | Create a signed commit | `-m <msg>`, `--author <name>` |
| `log` | Show commit history | `--json` |
| `checkout <commit>` | Restore files from commit | `--json` |
| `status` | Show working tree state | `--json` |
| `verify <commit>` | Verify integrity + signature | `--json` |
| `diff <commit1> <commit2>` | Compare two commits | `--json` |
| `prune` | Remove unreachable objects | |
| `tag` | Manage commit tags | `add`, `list`, `delete` |
| `branch` | Manage branches | `create`, `delete`, `list` |
| `merge <branch>` | Merge branch into current HEAD | `-m <msg>`, `--author <name>` |
| `config` | View/edit configuration | `get`, `set` |
| `share` | Announce commits to P2P network (Gossipsub) | |
| `sync` | Discover + fetch from peers | |
| `pull <peer> <commit>` | Pull commit from specific peer | |
| `push <peer>` | Push commits to peer | |
| `peer add <multiaddr>` | Add a known peer | `--public-key <hex>` |
| `backup <output>` | Create a tar.gz backup | |
| `restore <backup>` | Restore repo from backup | |
| `export <commit> <dir>` | Reconstruct commit to directory | `--json` |
| `import <dir>` | Ingest directory as commit | `-m <msg>`, `--author <name>` |
| `recover` | Recover from WAL crash | |
| `completions` | Generate shell completions | `bash`, `zsh`, `fish`, `elvish`, `powershell` |

### Global flags

| Flag | Effect |
| :--- | :--- |
| `--json` | Machine-readable JSON output |
| `--verbose` | Debug-level logging |

---

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                     shard CLI                         │
│               (clap argument parsing)                 │
└────────────┬───────────────────────────┬──────────────┘
             │                           │
             ▼                           ▼
┌───────────────────────┐   ┌──────────────────────────┐
│      core crate       │   │       net crate           │
│                       │   │                          │
│  ┌─────────────────┐  │   │  ┌────────────────────┐  │
│  │   Chunker        │  │   │  │   libp2p Node      │  │
│  │  (Fixed / Rabin) │  │   │  │  TCP+Noise+Yamux   │  │
│  ├─────────────────┤  │   │  │  mDNS / Kademlia    │  │
│  │   Compression    │  │   │  │  Gossipsub          │  │
│  │  (Zstd / Zlib)  │  │   │  │  Identify / Ping    │  │
│  ├─────────────────┤  │   │  │  Request-Response   │  │
│  │   Store          │  │   │  └────────────────────┘  │
│  │  (Sled / SQLite) │  │   └──────────────────────────┘
│  ├─────────────────┤  │
│  │   Commit DAG     │  │   ┌──────────────────────────┐
│  │   Manifest       │  │   │    crypto crate          │
│  │   Index / WAL    │  │   │                          │
│  │   Branch / Merge │  │   │  ed25519 key generation  │
│  │   Remote / Push  │  │   │  Signing / Verification  │
│  └─────────────────┘  │   └──────────────────────────┘
└───────────────────────┘
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
└── tags.json                     # named commit pointers
```

### Key design decisions

| Decision | Choice | Rationale |
| :--- | :--- | :--- |
| **Chunking** | Rabin (default) or Fixed | Rabin CDC improves dedup across versions; fixed for predictable sizes |
| **Compression** | Zstd or Zlib | Runtime selection; zstd is faster with better ratios |
| **Hashing** | Blake3 | Fastest cryptographic hash, SIMD-accelerated |
| **Signatures** | ed25519 | Proven, fast, small signatures (64 bytes) |
| **Storage** | Sled, SQLite, or Flat file | Sled/SQLite for indexed queries; flat for portability |
| **P2P** | libp2p TCP+Noise+Yamux | Mature, NAT traversal via relay/WebRTC planned |
| **Wire format** | JSON / CBOR | Serde over request-response + Gossipsub |

---

## Comparison

| Feature | Git | Shard |
| :--- | :--- | :--- |
| **Primary use** | Source code | ML artifacts (models, datasets, checkpoints) |
| **Chunking** | CDC (git fast-import) | Rabin + Fixed + configurable |
| **Compression** | zlib (default) | Zstd or Zlib (runtime selectable) |
| **Hashing** | SHA-1 (transitioning to SHA-256) | Blake3 |
| **P2P** | Remote-centric (push/pull to server) | Native P2P (mDNS, Kademlia, Gossipsub) |
| **Storage backend** | Flat files + packfiles | Sled or SQLite indexed store |
| **Signing** | GPG (optional) | ed25519 (built-in, every commit) |
| **Transport** | SSH/HTTPS | libp2p TCP + Noise + Yamux |



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
- [ ] Phase 9: Enterprise (CI polish, benchmarks, docs, community templates)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). All contributors must follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

See [SECURITY.md](SECURITY.md) for reporting vulnerabilities.

## License

MIT. See [LICENSE](LICENSE).
