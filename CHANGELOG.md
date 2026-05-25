# Changelog

## v1.1.0

### Bug fixes

- **HEAD corruption**: prevent HEAD corruption on failed checkout, fix backup/restore format mismatch
- **Flaky test**: `test_sync_auto_pull` ignored due to P2P race condition

### Documentation

- **Test report**: comprehensive report with 160+ scenarios, bugs, and performance benchmarks
- **Bug status**: Bug 1 and Bug 2 marked as fixed in REPORT.md

## v1.0.2

### Enterprise hardening

- **Crate metadata**: added `description`, `license`, `repository`, `homepage`, `keywords`, `categories` to all 5 workspace crates for crates.io publishing
- **Binary rename**: renamed package from `shard` → `shard-cli` to avoid crates.io collision; binary artifact preserved as `shard` via `[[bin]]` alias
- **Release workflow**: publish job now iterates all 5 crates in dependency order (`shard-crypto` → `shard-storage` → `shard-core` → `shard-net` → `shard-cli`)
- **Tracing migration**: replaced all ~84 `println!`/`eprintln!` calls in `core/` with `info!()`/`error!()` tracing macros

### Production hardening

- **Peer authentication** (#9): ed25519 challenge-response handshake, `authorized_keys` whitelist, backwards-compatible (no file = no auth)
- **Backup & recovery** (#10): `backup` (tar.gz), `export` (reconstruct files), `import` (ingest directory as commit), `restore` (extract backup)
- **Containerization** (#11): multi-stage Alpine Dockerfile, 3-node `docker-compose.yml`, Docker build in CI
- **Storage indexing** (#12): `objects.idx` index file for O(1) flat store chunk iteration with auto-rebuild on staleness
- **Tracing/observability** (#8): `tracing` + `tracing-subscriber` infrastructure with `--verbose` debug flag

### Push protocol

- **Push** (#7): DAG-walking object transfer to peer via `PutChunk` request-response; CLI: `shard push <peer>`

### Branching & merging

- **Branch management** (#5): `branch create`/`delete`/`list`, branch-aware `checkout`, detached HEAD support
- **Merge** (#6): union-of-manifests merge strategy, 2-parent merge commits, CLI: `shard merge <branch>`

### Advanced storage & chunking

- **Compression** (#1): runtime-selectable Zstd / Zlib / None, transparent compress/decompress in `add`/`verify`/`checkout`/`pull`, backwards-compatible with legacy manifests
- **Rabin chunking** (#3): variable-size content-defined chunking via buzhash rolling hash, configurable min/avg/max, same CLI ergonomics as fixed
- **Write-ahead log** (#4): crash-safe commits via JSON-lines WAL with HEAD+index backups, `shard recover` command
- **Directory recursion** (#2): recursive `walkdir`-based file discovery, auto-skips hidden files

## v0.2.0

- **Open-source infrastructure**: LICENSE, CODE_OF_CONDUCT, CONTRIBUTING, SECURITY
- **Cross-platform releases**: GitHub Actions release workflow (Linux, macOS, Windows)
- **Install scripts**: one-liner install via `scripts/install.sh` and `scripts/install.ps1`
- **Issue/PR templates**: standardized templates for bug reports, feature requests

## v0.1.0 — Initial implementation

- Local core: `init`, `add`, `commit`, `verify`, `log`, `checkout`, `status`, `config`, `tag`, `prune`
- P2P networking: `share`, `pull`, `sync`, `peer add`
- libp2p transport: TCP+Noise+Yamux, mDNS, Kademlia, Gossipsub, Identify
- Fixed 4 MiB chunking with Blake3 hashing
- ed25519 commit signing
- JSON protocol for request/response
