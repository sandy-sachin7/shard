# CLI Reference

Shard exposes a unified CLI with Git-like ergonomics. All commands support a `--json` flag for machine-readable output.

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
| `tag` | Manage commit tags | `add`, `list` |
| `branch` | Manage branches | `create`, `delete`, `list` |
| `merge <branch>` | Merge branch into current HEAD | `-m <msg>`, `--author <name>` |
| `config` | View/edit configuration | `get`, `set` |
| `share` | Announce commits to P2P network | `--json` |
| `sync` | Discover + fetch from peers | `--json` |
| `pull <peer> <commit>` | Pull commit from specific peer | `--json` |
| `push <peer>` | Transfer objects reachable from HEAD to a peer. The receiver can check out the reported commit ID; push does not move its refs. | `--json` |
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

Before a peer can push, add its public key on the receiving repo with `shard key add-authorized <public-key-hex>`. Repos without authorized keys allow P2P reads and reject P2P writes.

### Global flags

| Flag | Effect |
| :--- | :--- |
| `--json` | Machine-readable JSON output |
| `--log-format` | Log output format: `plain` (default) or `json` |
| `--verbose` | Debug-level logging |
