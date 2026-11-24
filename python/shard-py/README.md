# Shard Python Bindings

Native Python bindings (via PyO3) for [Shard](https://github.com/sandy-sachin7/shard) — the distributed VCS built for ML engineers by [Santhosh Sachin](https://github.com/sandy-sachin7).

## Installation

```bash
pip install shard-py
```

## Quick Start

```python
import shard

# Initialize a repository
shard.init()

# Track model weights
shard.add("model.pt")

# Commit after epoch
result = shard.commit("epoch 50", author="trainer@ml.dev")
print(result.commit_id)
```

## API Reference

### `shard.init(repo_path=None, private=False, db=None, compression=None, chunker=None, passphrase=None)`
Initialize a Shard repository.

- `repo_path` (str, optional): Path to repo. Defaults to current directory.
- `private` (bool, optional): Enable AES-256-GCM chunk encryption. Defaults to `False`.
- `db` (str, optional): Storage backend (`"flat"`, `"sled"`, `"sqlite"`). Defaults to `"flat"`.
- `compression` (str, optional): Compression codec (`"zstd"`, `"zlib"`, `"none"`). Defaults to `"zstd"`.
- `chunker` (str, optional): Chunking mode (`"fixed"`, `"rabin"`). Defaults to `"fixed"`.
- `passphrase` (str, optional): Passphrase for Argon2id + AES-256-GCM key protection.

### `shard.add(file_path, repo_path=None)`
Stage a file or directory for commit.

- `file_path` (str): Path to file or directory to track.
- `repo_path` (str, optional): Path to repo. Defaults to current directory.

### `shard.commit(message, repo_path=None, author=None)`
Commit staged changes with an Ed25519 signature.

- `message` (str): Commit message.
- `repo_path` (str, optional): Path to repo. Defaults to current directory.
- `author` (str, optional): Author string. Defaults to `"User <user@example.com>"`.

Returns a `CommitResult` object with `commit_id`, `message`, `author`, and `timestamp`.

### `shard.log(repo_path=None, limit=None)`
Get commit history.

- `repo_path` (str, optional): Path to repo.
- `limit` (int, optional): Maximum number of commits to return.

### `shard.status(repo_path=None)`
Get repository status as a JSON string (`branch`, `head`, `staged`).

### `shard.checkout(target, repo_path=None)`
Checkout a commit ID or branch name.

### `shard.verify(commit_id, repo_path=None, allow_unsigned=False)`
Verify commit content hashes, Merkle roots, and Ed25519 signature against local trust anchors.

## Example: Training Script Integration

```python
import shard
import torch

model = torch.nn.Linear(10, 2)
optimizer = torch.optim.SGD(model.parameters(), lr=0.01)

shard.init()

for epoch in range(100):
    # ... training code ...
    optimizer.step()

    if epoch % 10 == 0:
        # Save checkpoint
        torch.save(model.state_dict(), f"checkpoint_epoch_{epoch}.pt")

        # Track with Shard
        shard.add(f"checkpoint_epoch_{epoch}.pt")
        result = shard.commit(
            f"epoch {epoch}",
            author="trainer@ml.dev",
        )
        print(f"Committed: {result.commit_id}")
```

## Features

- **Native Rust extension**: Compiles directly against `shard-core` via PyO3 (no external CLI binary required)
- **Zero-config**: Works out of the box, no server setup required
- **Efficient**: Rabin CDC chunking deduplicates across training checkpoints
- **Secure**: Ed25519 commit signing and optional AES-256-GCM repo encryption

## Requirements

- Python 3.8+
- Rust toolchain 1.89+ (only when building from source)

## Installation from Source

```bash
git clone https://github.com/sandy-sachin7/shard
cd shard/python/shard-py
pip install .
```

## License

MIT
