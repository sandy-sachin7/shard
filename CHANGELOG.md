# Changelog

## v0.2.0 (unreleased)

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
- CBOR protocol for request/response
