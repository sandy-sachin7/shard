# Shard — Agent Guide

## Workspace (Cargo workspace, resolver "2")

| Crate | Path | Description |
|-------|------|-------------|
| `shard-cli` (bin) | `cmd/shard-cli/` | CLI entrypoint (clap derive, artifact: `shard`) |
| `shard-core` | `core/` | Chunker, store, manifest, index, commit DAG, top-level API (init/add/commit/verify/share/pull) |
| `shard-net` | `net/` | libp2p node (TCP+Noise+Yamux), protocol messages, mDNS+Kademlia+Gossipsub+Identify |
| `shard-crypto` | `crypto/` | ed25519 key generation, save/load (`secret.key` + `public.key`) |
| `shard-storage` | `storage/` | **Stub** — unused placeholder (only has `add(u64,u64)->u64`) |

## Commands

```sh
cargo build              # build workspace
cargo test               # run all tests (only chunker unit tests exist)
cargo install --path cmd/shard-cli  # install shard binary
```

No `tests/` integration tests exist yet. No CI workflows, no rustfmt/clippy configs.

## Architecture

- **Object storage**: `.shard/objects/<2‑char prefix>/<full hex hash>` — git-like layout, via `core/src/store.rs`
- **Chunking**: Fixed 4 MiB (`4 * 1024 * 1024`), Blake3 hashed, in `core/src/chunker.rs`
- **Commits**: serde JSON, `commit_id` field is **excluded from the hashed content** (placeholder). Signed via ed25519-dalek.
- **Networking**: libp2p 0.53 with TCP-only transport (`.with_quic()` commented out in `net/src/p2p.rs`)
- **CLI commands**: `Init`, `Add {path}`, `Commit { -m msg --author }`, `Verify {commit_id}`, `Peer Add {multiaddr}`, `Share`, `Pull {peer} {commit_id}`

## Key quirks

- `Cargo.lock` is gitignored
- `.shard/` and `ANTIGRAVITY.md` are gitignored
- `ANTIGRAVITY.md` is the design/spec document (gitignored but present)
- Only `core/src/chunker.rs` has unit tests
- `core/src/store.rs` `get_chunk` first reads the file directly in `verify` (bypasses store method) — `verify` accesses `.shard/objects/` manually
- The design doc (`ANTIGRAVITY.md`) references `nextest`, `sled`/`sqlite` storage, Rabin chunking, CBOR — none are implemented
