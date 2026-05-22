# Getting Started

## Install

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

**Build from source**

```bash
git clone https://github.com/sandy-sachin7/shard.git
cd shard
cargo build --release
./target/release/shard --help
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
