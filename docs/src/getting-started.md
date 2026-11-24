# Getting Started

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

**Build from source (Rust 1.89+)**

```bash
git clone https://github.com/sandy-sachin7/shard.git
cd shard
cargo build --release
./target/release/shard --help
```

---

## Quick Start

```bash
# Initialize a repository
shard init

# Add files (staged for commit)
shard add model.pt
shard add dataset/           # recursive directory add

# Commit with a message
shard commit -m "v1 checkpoint" --author "Alice <alice@ml.dev>"

# View history & working tree state
shard log
shard log --json             # machine-readable
shard status --porcelain

# Check out files from a commit
shard checkout <commit_id>

# Share with peers (prints your /ip4/.../tcp/<port>/p2p/<peer_id> multiaddr)
shard share --listen /ip4/0.0.0.0/tcp/9876

# Pull from a peer using its full multiaddr including /p2p/<peer_id>
shard pull /ip4/192.168.1.2/tcp/9876/p2p/<peer_id> <commit_id>

# Verify integrity and signature
shard verify <commit_id>

# Branching and merging
shard branch create experiment
shard checkout experiment
shard add model.pt
shard commit -m "experimental changes" --author "Alice <alice@ml.dev>"
shard checkout main
shard merge experiment -m "merge experiment" --author "Alice <alice@ml.dev>"

# Backup and recovery
shard backup /tmp/repo-backup.tar.gz
shard restore /tmp/repo-backup.tar.gz
shard export <commit_id> /tmp/reconstructed
shard import /tmp/datasets -m "imported dataset" --author "Alice <alice@ml.dev>"
```
