# Shard Python Bindings

Python wrapper for [Shard](https://github.com/sandy-sachin7/shard) — the distributed VCS built for ML engineers by [Santhosh Sachin](https://github.com/sandy-sachin7).

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
shard.commit("epoch 50", message="epoch 50")
```

## API Reference

### `shard.init(repo_path=None, private=False, db=None)`
Initialize a Shard repository.

- `repo_path` (str, optional): Path to repo. Defaults to current directory.
- `private` (bool, optional): Enable encryption. Defaults to False.
- `db` (str, optional): Storage backend ("flat", "sled", "sqlite"). Defaults to "flat".

### `shard.add(repo_path=None, file_path)`
Stage a file for commit.

- `repo_path` (str, optional): Path to repo. Defaults to current directory.
- `file_path` (str): Path to file to track.

### `shard.commit(repo_path=None, message, author=None)`
Commit staged changes.

- `repo_path` (str, optional): Path to repo. Defaults to current directory.
- `message` (str): Commit message.
- `author` (str, optional): Author string. Defaults to "User <user@example.com>".

Returns a `CommitResult` object with `commit_id`, `message`, `author`, and `timestamp`.

### `shard.log(repo_path=None, limit=None)`
Get commit history.

- `repo_path` (str, optional): Path to repo.
- `limit` (int, optional): Number of commits to return.

### `shard.status(repo_path=None)`
Get repository status.

### `shard.checkout(repo_path=None, target)`
Checkout a commit or branch.

### `shard.verify(repo_path=None, commit_id)`
Verify commit integrity and signature.

## Example: Training Script Integration

```python
import shard
import torch

model = torch.nn.Linear(10, 2)
optimizer = torch.optim.SGD(model.parameters(), lr=0.01)

for epoch in range(100):
    # ... training code ...
    optimizer.step()

    if epoch % 10 == 0:
        # Save checkpoint
        torch.save(model.state_dict(), f"checkpoint_epoch_{epoch}.pt")

        # Track with Shard
        shard.add(f"checkpoint_epoch_{epoch}.pt")
        result = shard.commit(
            message=f"epoch {epoch}",
            author="trainer@ml.dev"
        )
        print(f"Committed: {result.commit_id}")
```

## Features

- **Zero-config**: Works out of the box, no server setup required
- **Distributed**: Sync model checkpoints across machines via P2P
- **Efficient**: Rabin CDC chunking only transfers changed pieces
- **Secure**: Ed25519 commit signing and optional AES-256 repo encryption

## Requirements

- Python 3.8+
- Rust toolchain (for building from source)
- Shard binary installed and in PATH

## Installation from Source

```bash
git clone https://github.com/sandy-sachin7/shard
cd shard/python/shard-py
pip install .
```

## License

MIT