# Security Policy

## Supported Versions

Security fixes and advisories are actively maintained for the following versions:

| Version | Supported | Notes |
| :--- | :--- | :--- |
| `2.4.1` | ✅ Yes | Current patched production-hardened release |
| `2.4.0` | ❌ No | Upgrade to `2.4.1` for patched SQLite dependencies |
| `< 2.4.0` | ❌ No | Upgrade to `2.4.1` for `SHRD` v1 AAD encryption, 2-step P2P auth, and hardened restore/API |

---

## Reporting a Vulnerability

Shard handles high-value machine learning models, private datasets, and peer-to-peer network traffic. If you discover a security vulnerability, please **do not open a public GitHub issue or pull request**.

Report vulnerabilities privately via **GitHub Private Vulnerability Reporting**:

👉 **[Open a Private Security Advisory](https://github.com/sandy-sachin7/shard/security/advisories/new)**

### Response SLA

- **Acknowledgment**: Within 48 hours (2 business days).
- **Triage & Severity Assessment**: Within 5 business days, including a remediation timeline.
- **Coordinated Disclosure**: Fixes are released and published as a GitHub Security Advisory before public disclosure. Reporters are credited in release notes unless anonymity is requested.

---

## Threat Model & Trust Boundaries

| Boundary | Threat Mitigated | Mechanism in Shard |
| :--- | :--- | :--- |
| **Local Repository & Working Tree** | Corrupt/tampered artifacts, torn writes, path traversal on checkout/import, concurrent process corruption | Blake3 content addressing, manifest Merkle roots, CRC32 WAL (`wal.log`), atomic temp-file + `fsync` + rename, `safe_repo_join` confinement, `.shard.lock` OS advisory locking (`fs2`), `0o600` key file permissions |
| **Commit Provenance & Supply Chain** | Unsigned or forged commits, untrusted self-asserted public keys, expired/revoked keys | Mandatory Ed25519 commit & manifest signatures, trust-anchor verification against local keychain + `authorized_keys`, `key_id == blake3(public_key)` enforcement, signed key-rotation validity windows (`--allow-unsigned` required for legacy unsigned commits) |
| **P2P Network (libp2p)** | Unauthorized writes, replay/MITM auth attacks, SSRF via Gossipsub, resource exhaustion / DoS, malicious object IDs | Transport encryption via libp2p Noise/QUIC; 2-step domain-separated Ed25519 challenge-response (`shard-p2p-auth-v1:`, bound to client/server PeerIDs + 32-byte nonce, 30s TTL); `authorized_keys` write gate; SSRF-safe Gossipsub `address_book` allowlist; per-peer sliding-window `PeerRateLimiter`; strict 64-hex object ID validation + 65 MiB wire cap |
| **HTTP API (`shard serve`)** | Unauthenticated local/network access, timing attacks on tokens, cross-origin browser abuse, arbitrary file read via `/api/v1/add` | Constant-time bearer token auth (`subtle::ConstantTimeEq`) via `SHARD_API_TOKEN` or ephemeral 256-bit CSPRNG token; CORS disabled by default (`SHARD_API_CORS_ORIGINS` allowlist); strict `safe_repo_join` path confinement |
| **Backup & Restore** | Tar-slip path traversal (`../`), overwriting files outside `.shard/`, symlink/hardlink attacks | Strict `.shard/` prefix confinement on every tar entry, rejection of absolute paths, parent components, symlinks, hard links, and special device files, plus cleanup on failure |
| **HuggingFace Dataset Export** | Accidental leakage of private keys, WALs, or lock files; incomplete SQLite snapshots | Mandatory allowlist staging + preflight check aborting on `secret.key`, `repo.key`, `archive/`, `wal.log`, `partial/`, `repo.lock`, `.shard.lock`; exclusive lock + SQLite `PRAGMA wal_checkpoint(TRUNCATE)` / Sled flush prior to upload |

---

## Cryptographic Primitives

| Purpose | Algorithm / Construction | Library |
| :--- | :--- | :--- |
| Content addressing & Merkle roots | **BLAKE3** (256-bit) | `blake3` |
| Commit, manifest, rotation & P2P auth signatures | **Ed25519** | `ed25519-dalek` |
| Chunk encryption at rest (`--private`) | **AES-256-GCM** with `chunk_id` as AAD (`SHRD` v1 envelope) | `aes-gcm` |
| Passphrase key derivation (`secret.key`, `repo.key`) | **Argon2id** (16-byte random salt) + **AES-256-GCM** | `argon2`, `aes-gcm` |
| In-memory secret hygiene | Automatic zero-on-drop (`Zeroizing<T>`) | `zeroize` |
| Bearer token comparison | Constant-time equality | `subtle` |
| P2P transport security | **Noise** (TCP) / **TLS 1.3** (QUIC) via libp2p | `libp2p` |
| WAL entry integrity | **CRC32** | `crc32fast` |

## Zero Telemetry

Shard collects and transmits **zero telemetry**. Network traffic only occurs when you explicitly invoke P2P commands (`share`, `sync`, `pull`, `push`, `relay`) or HuggingFace sync (`hf push`, `hf pull`).
