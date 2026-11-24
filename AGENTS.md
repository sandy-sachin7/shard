# Shard — Agent Guide

## Workspace (Cargo workspace, resolver `"2"`, `v2.4.1`)

| Crate | Path | Description |
|-------|------|-------------|
| `shard-cli` (`shard` bin) | `cmd/shard-cli/` | CLI entrypoint (`clap` derive), stderr tracing, clean stdout data/JSON |
| `shard-core` | `core/` | Modular VCS engine: `repo`, `auth`, `backup`, `p2p_sync`, `hf`, `api`, `gc`, `wal`, `ops`, `branch`, `chunker`, `chunk_codec`, `compression`, `encryption`, `keychain`, `store/*` |
| `shard-net` | `net/` | `libp2p` 0.57 node (TCP+QUIC+Noise+Yamux, mDNS, Kademlia, Gossipsub, Relay, DCUtR, AutoNAT) + Ed25519 challenge-response auth |
| `shard-crypto` | `crypto/` | Ed25519 key generation, Argon2id + AES-256-GCM passphrase encryption, `0o600` atomic secret persistence, `zeroize` |
| `shard-storage` | `storage/` | `StorageBackend` trait + `SledBackend` and pooled WAL-mode `SqliteBackend` with key-only `iter_keys()` |
| `shard-tests` | `tests/` | Workspace-level library integration tests |
| `shard-py` (standalone) | `python/shard-py/` | PyO3 native Python bindings calling `shard-core` directly with GIL release |

## Verification Commands

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

## Architecture Highlights

- **Content-addressed storage**: Flat (`objects/<2-hex>/<64-hex>`, atomic write + fsync), Sled, or pooled SQLite (WAL + `busy_timeout=5000`). All object IDs strictly validated as 64 lowercase hex characters.
- **Self-describing chunk envelopes (`SHRD` v1)**: Encodes compression & encryption flags with AAD bound to `chunk_id` (backward compatible with legacy raw blobs). Enables safe dedup, `push`/`pull` resume across `zstd`/`zlib`/`--private`, and wire-level integrity verification.
- **FastCDC chunking**: 64-bit Gear hash with dual normalized masks (`mask_s`/`mask_l`) and cursor-based buffer (shift-invariant deduplication).
- **Concurrency & durability**: Per-repo in-process `RwLock` + OS-level `<repo_root>/.shard.lock` (`fs2`), CRC32-checksummed WAL backing up `HEAD`, active branch ref, and `index`, plus cross-platform atomic write-fsync-persist for all metadata.
- **Security**:
  - Ed25519 challenge-response P2P auth with 32-byte `OsRng` nonces, 30s TTL, dual-PeerId domain separation, and `authorized_keys` enforcement.
  - Strict `safe_repo_join` path containment & `validate_branch_name` traversal protection.
  - Hardened HTTP API (`shard serve`): constant-time bearer auth (`SHARD_API_TOKEN`), no permissive CORS, `spawn_blocking` offload.
  - `shard hf push` stages only sanitized public artifacts and refuses to upload private keys (`secret.key`, `repo.key`, `archive/`), WALs, or lock files.
