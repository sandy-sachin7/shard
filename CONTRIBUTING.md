# Contributing to Shard

Thanks for your interest! We welcome contributions of all kinds — features, bug fixes, docs, benchmarks, and issue reports.

## Quick Start

```bash
git clone https://github.com/sandy-sachin7/shard.git
cd shard
cargo build
cargo test
```

## What We Need Help With

- **New storage backends**: SQLite, S3-compatible object stores
- **Chunker algorithms**: CDC variants (Rabin, Buzhash, FastCDC)
- **Network transport**: WebRTC, QUIC, relay nodes for NAT traversal
- **Packaging**: Homebrew, Scoop, Nix, Docker
- **Documentation**: clearer error messages, more examples, protocol spec

## Pull Request Process

1. Run `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo audit` before submitting
2. Keep PRs focused on one concern — split large changes into multiple PRs
3. Update the README if your change affects the CLI or user-facing behavior
4. Add tests for new functionality

## Design Principles

- **Local-first** — no cloud dependency, data lives on your machine
- **Content-addressed** — deduplication via hashing, integrity via verification
- **P2P by default** — distribute directly between peers, no central server
- **Fail gracefully** — malformed data is skipped with a warning, never panic
- **Deterministic outputs** — `--json` flags on all commands for scripting

## Code of Conduct

All contributors must abide by our [Code of Conduct](CODE_OF_CONDUCT.md).
