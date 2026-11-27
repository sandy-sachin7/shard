# Changelog

## v2.6.0 (2026-11-28) — Descriptor-Pinned Filesystem Ops & Hard-Kill Crash Recovery

### Added
- **Descriptor-Pinned Repository Filesystem (`PinnedRepoDir` in `core/src/fs_util.rs`)**: Pins the repository root with `O_DIRECTORY | O_CLOEXEC` on Unix and walks every relative path component-by-component with `openat(..., O_NOFOLLOW)`, `mkdirat`, `renameat`, `unlinkat`, and `fstatat(..., AT_SYMLINK_NOFOLLOW)` to eliminate TOCTOU symlink races during checkout staging, backup, swap, rollback, and cleanup.
- **CRC32-Verified Checkout Journal & Blake3 Backup Verification**: Wraps `.shard/checkout_journal.json` in a versioned CRC32 envelope (`CheckoutJournalEnvelope`) and records `backup_blake3` and `expected_blake3` on every journal entry so corrupted journals and bit-rotted backups fail loudly instead of silently restoring bad data.
- **Real Hard-Kill Subprocess & Symlink-Refusal Resilience Tests (`tests/resilience.rs`)**: Added tests spawning real OS child processes that terminate with `exit(137)` (zero Rust `Drop` destructors) mid-swap and before `HEAD` update, plus CRC32 journal corruption, backup Blake3 bit-rot, and Unix `O_NOFOLLOW` symlink-refusal tests.

## v2.5.2 (2026-11-25) — Windows Checkout Backup Fsync Fix

### Fixed
- **Windows Backup Durability Handle (`core/src/repo.rs`)**: Open checkout backup files with write access (`OpenOptions::new().write(true)`) before calling `sync_all()`, fixing `Access is denied (os error 5)` from Windows `FlushFileBuffers` during multi-file checkout.

## v2.5.1 (2026-11-25) — Durable Crash-Atomic Checkout Journal

### Fixed
- **Crash-Atomic Multi-File Checkout Journal (`.shard/checkout_journal.json`)**: Separated `shard checkout` into strict phases: stage and hash-verify every manifest, back up every existing target file with `sync_all()` before mutating any working-tree file, persist an atomic fsynced checkout journal (`Swapping` -> `UpdatingHead`), swap files, commit `HEAD`, and remove backups and journal last.
- **Explicit Rollback Error Propagation**: Failed swaps and failed `HEAD` updates roll back all previously replaced files via idempotent copy-and-replace from durable backups, restore `HEAD`, surface any rollback errors via `ShardError::CheckoutRollbackFailed`, and keep `.shard/checkout_journal.json` on disk when rollback encounters an error so `shard recover` can retry.
- **Automatic & Explicit Crash Recovery**: `shard recover` (and subsequent write operations `add`, `commit`, `checkout`) replays `.shard/checkout_journal.json`, rolling back interrupted `Swapping` checkouts or rolling forward `UpdatingHead` checkouts cleanly.
- **Boundary Fault-Injection Tests (`tests/resilience.rs`)**: Added deterministic tests covering backup failure on file 2, mid-swap failure after file 1 was overwritten, `HEAD`-update failure, simulated mid-swap crash recovery, simulated `UpdatingHead` crash recovery, and rollback error propagation.

## v2.5.0 (2026-11-25) — Transactional Checkout, FastCDC Naming & Empirical Checkpoint Benchmarks

### Added
- **Transactional Multi-File Checkout**: `shard checkout` now stages and verifies every manifest into same-directory temp files before overwriting any working-tree file, snapshots existing targets for rollback on persist errors, fsyncs touched parent directories, and updates `HEAD` last. A corrupt or missing chunk on file N of M leaves files 1..N-1 and `HEAD` untouched.
- **FastCDC Naming (`--chunker fastcdc`)**: Renamed the Gear-hash content-defined chunker from `RabinChunker` / `--chunker rabin` to `FastCdcChunker` / `--chunker fastcdc`, keeping `RabinChunker`, `Chunker::new_rabin`, `validate_rabin_params`, and `--chunker rabin` as backwards-compatible aliases.
- **Empirical ML Checkpoint Deduplication Benchmark (`benchmarks/bench_checkpoints.py`)**: Measured real IEEE-754 float32 safetensors-style checkpoints across full fine-tuning (0.0% chunk dedup), frozen 90% backbone + 10% updated head (89.0% dedup), and 1 MiB adapter prepend over a frozen backbone (54.5% dedup vs 0.0% fixed), checked into `benchmarks/checkpoint_dedup_results.json`.
- **Resilience & Property Test Suite (`tests/resilience.rs`)**: Added FastCDC roundtrip/shift-invariance property tests, `SHRD` v1 envelope bit-flip/truncation/AAD-swap rejection tests, torn/corrupt CRC32 WAL recovery tests, and multi-file transactional checkout rollback tests.

### Changed
- **Documentation Accuracy**: Replaced "Rabin CDC" with "FastCDC", removed "Enterprise" headings, updated the comparison table to note HuggingFace Hub uses FastCDC (Xet), and documented where deduplication helps (frozen backbones, adapters, offset-shifted checkpoints, datasets) vs where it does not (full fine-tune epochs).

## v2.4.1 (2026-11-25) — Dependency Security Patch

- Upgraded `rusqlite` to 0.42 and `r2d2_sqlite` to 0.37, moving `libsqlite3-sys` to 0.40 and clearing `RUSTSEC-2026-0459`.
- Refreshed the lockfile and audit configuration comment.

## v2.4.0 (2026-11-24) — Production Security, Durability & Correctness Hardening

### Security & Trust Boundaries
- **P2P Ed25519 challenge-response authentication**: Restored real challenge-response handshake using fresh 32-byte `OsRng` nonces with 30s TTL, domain separation bound to both client and server `PeerId`s, `authorized_keys` enforcement, and automatic session cleanup on `ConnectionClosed`.
- **Strict ID & path validation**: Added `validate_object_id` (64 lowercase hex chars) at every `Store` backend, `PartialTransfer`, and P2P handler; added `safe_repo_join` preventing absolute paths, `..`, `.shard`/`.git`, and symlink escapes across `checkout`, `export`, `pull`, and `/api/v1/add`; added `validate_branch_name` blocking `../` traversal while supporting hierarchical branch names like `feature/foo`.
- **HuggingFace push key protection**: `shard hf push` now stages only safe public repository artifacts (including `objects.db` for Sled/SQLite repos) into an isolated temp directory with a mandatory pre-flight assertion refusing `secret.key`, `repo.key`, `keys/archive/`, `wal.log`, `partial/`, and `.shard.lock`; passes tokens via `HF_TOKEN` env var instead of CLI flags.
- **Hardened HTTP API (`shard serve`)**: Removed `CorsLayer::permissive()`; added constant-time bearer token auth (`SHARD_API_TOKEN` or ephemeral `OsRng` token); confined `/api/v1/add` inside the repository; fixed `/api/v1/commit` to return real `commit_id` and `/api/v1/status` to load `.shard/index`; offloaded blocking operations via `tokio::task::spawn_blocking`.
- **Signature trust & key hardening**: `verify` and `pull`/`sync` require valid signatures (with `--allow-unsigned` opt-in), verify `key_id == blake3(public_key)`, and anchor trust to local keychain + `authorized_keys`; all secret keys written atomically with `0o600` permissions; `repo.key` wrapped with Argon2id + AES-256-GCM when passphrase is set; `key rotate` supports encrypted keys; secret buffers zeroized with `zeroize`; AEAD AAD binds chunk hashes; removed panicking `RepoCipher::key_bytes()`; `SHARD_PASSPHRASE` env var supported across all commands.
- **Dependency upgrades**: Upgraded `libp2p` to `0.57` (and transitive `hickory`, `rustls-webpki`, `ring`, `bytes`, `rand`, `time`), eliminating active security/soundness advisories and pruning `.cargo/audit.toml`.

### Core Storage, Durability, Memory & Correctness
- **Atomic writes & crash-safe WAL**: Added `atomic_write` (`write -> sync_all -> rename -> dir fsync`) across `FlatStore`, `Index`, refs, `HEAD`, `tags.json`, and `config.json`. WAL now backs up `HEAD`, active branch ref (`refs/heads/<branch>`), and `index` with per-record CRC32 checksums, `sync_all()`, and graceful recovery from truncated trailing records.
- **Real cross-process locking**: Replaced polling `OpQueue` with OS-level advisory locking (`fs2` on `<repo_root>/.shard.lock`) combined with in-process `RwLock` across all operations.
- **Eliminated OOM paths**: Replaced 1 GiB `zstd::bulk::decompress` and unbounded `zlib` decompression with bounded streaming decompression (64 MiB default cap); streamed `checkout`, `export`, and `pull` chunk-by-chunk via `BufWriter`; streamed `push` object-by-object from `Store`; added key-only `StorageBackend::iter_keys()` for Sled/SQLite GC.
- **Self-describing `SHRD` v1 chunk envelopes**: Fixed `push` rejection and `pull` resume corruption for `zstd`/`zlib`/`--private` repos and prevented silent corruption when compression/privacy flags change.
- **3-way LCA merge**: Rewrote `merge()` with DAG Lowest Common Ancestor computation, full `FileManifest` preservation (compression, size, content_type, merkle_root), and explicit conflict detection.
- **Fail-closed GC**: Deduplicated `prune` -> `gc::gc()`; added recursive branch scanning for slashed branches, `KeyRotation` and `partial/` reachability marking, iterative DAG traversal, RAII panic-safe guard, and fail-closed abort on any mark-phase error.
- **Backend & chunker fixes**: Removed buggy `objects.idx` from `FlatStore`; enabled SQLite WAL + `synchronous=NORMAL` + `busy_timeout=5000` on every pooled connection; reused open `Store` in `import`/`share`/`sync` to fix Sled double-open crashes; replaced broken rolling hash with true shift-invariant FastCDC (64-bit Gear hash + dual normalized masks).
- **Commit snapshot semantics & `import()`**: Commits now carry forward unmodified manifests from parent commits; `import()` preserves source directory hierarchy.

### Modularity, Bindings, CLI & Repo Hygiene
- Decomposed 3,566-LOC `core/src/lib.rs` into focused submodules (`repo`, `auth`, `backup`, `p2p_sync`, `hf`, `chunk_codec`, `fs_util`, `error`).
- Fixed `SHARD_*` env var key mapping (preserving underscores) and wired `validate_config()` (with `zlib` instead of `gzip`).
- Rewrote `python/shard-py` to invoke `shard-core` directly with GIL release (`py.allow_threads`).
- Routed CLI diagnostics strictly to `stderr` while keeping clean data/JSON on `stdout`; fixed `key verify --json` non-zero exit code on failure.
- Un-ignored and stabilized `test_three_node_pull` and `test_sync_auto_pull`; added comprehensive security, P2P auth, merge, GC, and HTTP API tests.
- Hardened `Dockerfile` (non-root user), `docker-compose.yml`, and `scripts/install.sh` (`mktemp -d` + `trap` + macOS `shasum`); cleaned root clutter.

## v2.3.0

- Added HuggingFace dataset push/pull (`shard hf push` / `shard hf pull`).
- Added `--porcelain` flag to `shard status`.
- Improved `.shardignore` filtering and hidden file handling.

## v2.2.1

### Bug fixes

- **Flaky test**: `test_three_node_pull` marked `#[ignore]` due to P2P race condition in test environment.

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
