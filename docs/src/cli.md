# CLI Reference

Shard exposes a unified CLI with Git-like ergonomics. All commands support a `--json` flag for machine-readable output.

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
| `share` | Announce commits to P2P network | |
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
