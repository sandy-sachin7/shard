# Contributing to Shard

Thanks for your interest! We welcome contributions of all kinds — features, bug fixes, security reviews, documentation, benchmarks, and issue reports.

## Quick Start

```bash
git clone https://github.com/sandy-sachin7/shard.git
cd shard
cargo build
cargo test --workspace --locked
```

> **MSRV:** Rust **1.89+** is required.

## What We Need Help With

- **Cloud-hybrid storage adapters**: S3, GCS, and Azure Blob remote object-store backends
- **Delta compaction**: Packfile / cross-chunk delta compression for tightly correlated checkpoints
- **Transport & browser clients**: WebRTC transport and browser-based repository inspection
- **Packaging & distribution**: Homebrew formula, Nix flake, Scoop manifest, distro packages
- **Security & reliability**: Fuzzing harnesses (tar restore, wire CBOR, `SHRD` v1 codec), chaos/crash tests
- **Documentation & examples**: End-to-end PyTorch / HuggingFace / JAX workflow guides and benchmarks

## Pull Request Checklist

Before opening a pull request, run the full verification suite:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo test --workspace --locked
cargo audit
```

If you modified Python bindings (`python/shard-py/`), also verify:

```bash
PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo check --manifest-path python/shard-py/Cargo.toml
```

### Guidelines

1. Keep PRs focused on one concern — split large changes into reviewable PRs.
2. Never introduce `.unwrap()` or `.expect()` in non-test production code under `src/`.
3. Add unit and/or integration tests for all new functionality and bug fixes.
4. Update `README.md` and `docs/src/` whenever CLI flags, wire formats, or user-facing behavior change.

## Design Principles

- **Local-first** — no cloud dependency; repositories work fully offline.
- **Content-addressed & verifiable** — Blake3 hashing, Merkle roots, and Ed25519 provenance on every commit.
- **P2P by default** — distribute directly between peers with authenticated, rate-limited libp2p streams.
- **Fail safely** — malformed or untrusted inputs return structured errors; never panic or corrupt repository state.
- **Scriptable** — machine-readable `--json` outputs on stdout, diagnostic logs strictly on stderr.

## Code of Conduct

All contributors must abide by our [Code of Conduct](CODE_OF_CONDUCT.md).
