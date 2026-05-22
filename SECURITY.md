# Security Policy

## Reporting a Vulnerability

Shard is a P2P networking tool that handles potentially sensitive ML artifacts. While we follow security best practices, if you discover a vulnerability, please **do not open a public issue**.

Instead, email the maintainers directly or open a draft security advisory on GitHub.

We will respond within 48 hours and coordinate a fix before public disclosure.

## What to expect

- You will receive acknowledgment of your report within 2 business days
- We will provide a timeline for the fix and release
- You will be credited in the release notes (unless you prefer anonymity)

## Security features

- **Signed commits**: every commit is ed25519-signed for provenance with embedded public key
- **Peer authentication**: optional challenge-response ed25519 auth via `authorized_keys` file; whitelist only peers whose public keys are on file
- **Peer identity**: libp2p Noise handshake with ed25519 keys for transport-level encryption
- **Content verification**: Blake3 hash verification on every chunk read/write with optional decompress→rehash detection
- **No telemetry**: zero data transmitted outside the P2P network; no external telemetry, analytics, or phone-home
- **Local-first by default**: `--private` flag disables P2P sharing at init time; private repos never announce on the network
