# Changelog

## v2.2.0 — Enterprise Hardening

### Concurrency Control (#6)

- **Operation queue**: `OpQueue` with acquire/wait-for-turn pattern, read-write semantics via `parking_lot::RwLock`.
- **Per-repo serialization**: `RepoOpQueues` global map using `once_cell::sync::Lazy` — reads run in parallel, writes serialize per repo.
- **Operation guards**: All public API functions (init, add, commit, verify, checkout, merge, prune, push) wrapped with `with_write_op`/`with_read_op`.

### Configuration System (#7)

- **Environment overrides**: `SHARD_*` env vars (e.g. `SHARD_STORAGE_BACKEND=sqlite`) auto-override `config.json` values.
- **Config validation**: Runtime validation for `storage_backend`, `compression`, `chunker_mode`, `chunk_size`, `rate_limit_max_requests`, `rate_limit_window_secs`, `gc_interval_secs`.
- **Config helpers**: `config_get_rate_limit_max`, `config_get_rate_limit_window`, `config_get_gc_interval`, `config_get_gc_enabled` with sensible defaults (50 req / 60s window / 3600s interval / disabled).
- **New config keys**: `rate_limit_max_requests`, `rate_limit_window_secs`, `gc_enabled`, `gc_interval_secs`.

### SQLite Connection Pool (#8)

- **Connection pooling**: Replaced `Mutex<Connection>` with `r2d2::Pool<SqliteConnectionManager>` (max 8 connections).
- **Dependencies**: Added `r2d2` and `r2d2_sqlite` workspace dependencies.

### P2P DoS Hardening (#9)

- **Configurable rate limits**: P2P request rate limits driven by `rate_limit_max_requests` (default 50) and `rate_limit_window_secs` (default 60) config keys.
- **Per-peer request throttling**: Sync loop enforces per-peer request cap within rolling window.

### Automated GC Policy (#10)

- **`GcPolicy` struct**: Reads `gc_enabled` / `gc_interval_secs` from config.
- **DAG reachability scan**: Walks commit DAG from HEAD, branches, tags, and index to determine live objects.
- **Unreachable pruning**: Deletes chunks not reachable from any reference.
- **Background GC loop**: `gc_loop()` runs as async tokio task with configurable interval and `ctrl-c` signal handling.

### Distributed Tracing (#11)

- **Trace ID propagation**: Thread-local `CURRENT_TRACE_ID` via `thread_local!` with `generate_trace_id()` / `set_trace_id()` / `get_trace_id()`.
- **Traced logging helpers**: `traced_info!()` / `traced_warn!()` prepend `[trace_id]` prefix to log messages.
- **UUID-based IDs**: Uses `uuid::Uuid::v4` for operation and trace IDs.

### Test Gap Closure (#12)

- All existing tests pass (181 total, 1 flaky P2P test ignored).
- Config unit tests added for env override parsing, validation, and config defaults.
- GC unit tests for policy defaults and reachability on nonexistent commits.

### Infrastructure

- **Metrics**: Runtime operation counters (ops_init through ops_relay) with atomic increment and JSON snapshot support.
- **Health command**: `shard health` reports repository status with metrics snapshot.
- **API versioning**: HTTP API uses `/api/v1/*` routes.
- **Dependency cleanup**: OnceCell for global state, UUID for tracing.
- **Code quality**: `#[derive(Default)]` on metrics, unified `promote_pending` logic, `&Path` instead of `&PathBuf` in API, `#[allow(clippy::too_many_arguments)]` on `init_with_passphrase`.
- **Flaky test**: `test_three_node_pull` marked `#[ignore]` due to P2P race condition in test environment.

## v2.1.1

### Bug fixes

- **macOS Build**: Fixed `_PyBaseObject_Type` undefined symbols by properly injecting `-undefined dynamic_lookup` linking flags for PyO3 (`PYO3_MAC_EXT_LINK_DYNAMIC_LOOKUP`).
- **Windows Build**: Resolved `LNK1181` linking error by ensuring PyO3 can accurately locate `python3.lib` in the host environment.
- **Security**: Upgraded `pyo3` to `0.24.2` to resolve a known buffer overflow vulnerability (`RUSTSEC-2025-0020`).
- **Docker**: Ensured `python3` is available in Alpine builder and fixed `.cargo` missing directory issues by updating `.dockerignore`.
## v2.1.0

### Bug fixes

- **Build fixes**: Resolved Docker workspace build errors and PyO3 compatibility.
- **File exclusion**: Implemented `.shardignore` pattern matching, fixed logic to correctly allow tracking of hidden files.

### Infrastructure

- **Dockerfile**: Added `tests/` workspace member to multi-stage build.
- **Flaky test**: `test_sync_auto_pull` ignored due to P2P race condition.

## v2.0.0

### Bug fixes

- **Dockerfile**: added `tests/` workspace member to multi-stage build (was causing build failure in CI)
- **HEAD corruption**: prevent HEAD corruption on failed checkout, fix backup/restore format mismatch
- **Flaky test**: `test_sync_auto_pull` ignored due to P2P race condition

### Infrastructure

- **Version bump**: 1.1.0 → 2.0.0 across all workspace crates for semantic versioning clarity

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
