# CLI Reference

Shard exposes a unified CLI with Git-like ergonomics. Most data and status subcommands accept `--json` for machine-readable `stdout` output, while diagnostic logs are always written to `stderr`.

| Command | What it does | Key flags |
| :--- | :--- | :--- |
| `init` | Initialize a repository | `--private`, `--db flat\|sled\|sqlite`, `--compression zstd\|zlib\|none`, `--chunker fixed\|rabin`, `--chunk-size <bytes>`, `--passphrase`, `--json` |
| `add <path>` | Stage files or directories for commit | `--json` (recursive for directories; honors `.shardignore`) |
| `commit` | Create a signed commit | `-m <msg>`, `--author <name>`, `--json` |
| `log` | Show commit history | `--json` |
| `checkout <target>` | Restore files from commit ID or branch | `--json` |
| `status` | Show working tree state | `--json`, `--porcelain` |
| `verify <commit>` | Verify integrity, Merkle roots, and signature | `--allow-unsigned`, `--json` |
| `diff <commit1> [commit2]` | Compare two commits | `--json` |
| `prune` | Remove unreachable objects | `--json` |
| `tag` | Manage commit tags | `add <name> <commit>`, `list` |
| `branch` | Manage branches | `create <name> [commit]`, `delete <name>`, `list` |
| `merge <branch>` | Merge branch into current HEAD | `-m <msg>`, `--author <name>`, `--json` |
| `config` | View/edit configuration | `get [key]`, `set <key> <value>` |
| `share` | Announce and serve commits on P2P network | `--listen <multiaddr>`, `--json` |
| `sync` | Discover and auto-pull updates from known peers | `--json` |
| `pull <peer> <commit>` | Pull commit from specific peer (`/p2p/<peer_id>`) | `--json` |
| `push <peer>` | Transfer objects reachable from HEAD to a peer (does not move receiver refs) | `--json` |
| `peer add <multiaddr>` | Add a known peer | `--public-key <hex>`, `--json` |
| `hf push <dataset_id>` | Checkpoint and upload sanitized `.shard` to HuggingFace Datasets | `--token <tok>` (prefer `HF_TOKEN`), `--json` |
| `hf pull <dataset_id>` | Download `.shard` repository from HuggingFace Datasets | `--token <tok>` (prefer `HF_TOKEN`), `--json` |
| `backup <output>` | Create a `.tar.gz` backup | `--json` |
| `restore <backup>` | Restore `.shard/` safely from backup | `--json` |
| `export <commit> <dir>` | Reconstruct commit to directory | `--json` |
| `import <dir>` | Ingest directory as commit | `-m <msg>`, `--author <name>`, `--json` |
| `recover` | Recover from WAL crash | `--json` |
| `health` | Show repository diagnostics + metrics | `--json` |
| `serve` | Start authenticated HTTP API server | `--addr <host:port>`, `--json` |
| `unlock` | Verify passphrase against repository keys | `--passphrase` (prefer `SHARD_PASSPHRASE`) |
| `relay` | Start P2P Circuit Relay v2 node | `--listen <multiaddr>`, `--json` |
| `transfer` | Manage resumable P2P partial transfers | `list [--json]`, `remove <commit_id>` |
| `key` | Manage signing keys & authorized peers | `rotate`, `list [--json]`, `verify [--json]`, `add-authorized <hex>`, `remove-authorized <hex>`, `list-authorized [--json]` |
| `completions <shell>` | Generate shell completions | `bash`, `zsh`, `fish`, `elvish`, `powershell` |

Before a peer can push, add its public key on the receiving repo with `shard key add-authorized <public-key-hex>`. Repos without `authorized_keys` allow P2P reads and reject P2P writes.

## Global Flags

| Flag | Effect |
| :--- | :--- |
| `--log-format <plain\|json>` | Log output format on `stderr`: `plain` (default) or `json` |
| `-v`, `--verbose` | Enable debug-level logging on `stderr` |
