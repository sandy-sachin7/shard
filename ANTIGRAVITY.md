# Shard — P2P Model Version Control — Complete Build Artifact

> Project name: **Shard**
>
> Tagline: *Distributed, content‑addressed version control for large ML artifacts — no cloud bills, no central bottlenecks.*

---

## 0. One‑line system goal

Build a protocol‑first, local‑first, peer‑to‑peer version control system for machine learning artifacts (models, datasets, checkpoints) that runs entirely from developer machines and community hosts without requiring paid cloud storage.

---

## 1. Product identity & CLI contract

**Executable name:** `shard`

**High‑level UX:** Git‑like ergonomics, explicit `add/commit/push/pull/sync/verify` flows, deterministic outputs, machine‑parseable JSON flags.

Core commands (final CLI spec):

```
shard init [--private]
shard config set <key> <value>
shard add <file|dir>
shard commit -m "message" --author "Name <email>"
shard tag add <name> <commit>
shard share                       # announce commits to peers
shard sync                        # discover announcements + fetch missing
shard pull <peer-multiaddr> <commit>
shard checkout <commit>           # materialize artifacts
shard verify <commit>             # integrity + signature check
shard peer add <multiaddr>
shard log [--json]
shard status
shard prune                       # cleanup unreachable objects
```

All commands support `--json` for machine parsing and return nonzero exit codes on failure.

---

## 2. Core technical stack & reasons

* **Language:** Rust (recommended) — performance, safety, single static binaries, mature async.
* **Async runtime:** Tokio
* **P2P:** rust‑libp2p (DHT + PubSub + streams + WebRTC when needed)
* **Hashing:** Blake3 (primary) with SHA256 compatibility mode
* **Signature:** ed25519 (per‑user keypair) + optional repo rotated keys
* **Storage:** local FS (objects/), index in `sled` or `sqlite` (configurable)
* **Serialization:** Serde JSON (canonical key ordering) and optional CBOR for compact storage
* **Chunking:** fixed 4MiB default + optional Rabin rolling chunker for dedupe
* **Build tooling:** cargo, cargo‑nextest, goreleaser/cargo‑deb for packaging

---

## 3. Data model (deterministic and canonical)

* **Blob (chunk):** raw bytes saved in `objects/<2prefix>/<hash>`; metadata: size, offset. Hash = Blake3(chunk).
* **Manifest:** artifact descriptor: filename, content type, compression flag, chunk list (ordered), merkle root, size, created_by, created_at.
* **Commit node:** JSON with `commit_id` (hash of canonical commit JSON), `parents:[]`, `manifests:[]`, `author`, `message`, `timestamp`, `signature`.
* **Tag:** pointer name → commit_id.
* **DAG semantics:** Directed acyclic graph, parents may be multiple; no cycles allowed; canonical serialization ensures deterministic commit ids.

All metadata is canonicalized (sorted keys) and signed using ed25519. Keys are stored in `~/.shard/keys` and per‑repo config references them.

---

## 4. Wire protocol & primitives

**Discovery:**

* Primary: libp2p DHT + kademlia.
* LAN fallback: mDNS.
* Manual bootstrap: `shard peer add <multiaddr>`.

**Announcements:**

* Topic: `shard:ann`. Simple PubSub message containing `commit_id`, minimal manifest summary, repo name, and peer multiaddr.

**Fetch flow:**

1. Peer sees announcement → requests manifest via DHT or direct stream to announcer.
2. Manifest returned (signed).
3. Peer compares chunk list to local index → requests missing chunks via parallel piece requests (libp2p streams).
4. Chunks transferred with piece headers `{hash, offset, size}` and signed payload.
5. On complete, client assembles artifact and verifies Merkle root and final digest.

**Resilience:** parallel downloads, chunk retries, resumable transfer (persist partial chunks to `.partial/`).

**Security model:**

* Metadata signed; payloads integrity verified; optional encryption with per‑repo symmetric keys (stored on user machines or shared via OOB channels).
* Key rotation via a signed revocation commit in the DAG.

---

## 5. Phase roadmap — concrete milestones

### Phase 0 — Design & scaffolding

* Deliverables: protocol spec, canonical JSON schemas, minimal CLI spec, test harness plan.
* Single milestone commit: `chore(design): add protocol spec and canonical schemas`.

### Phase 1 — Local core

* Implement: `shard init`, local key generation, `add` (chunker + store), `commit` (manifest + commit DAG), `verify` (local).
* Tests: unit tests for chunker, hash vectors, manifest roundtrips.
* Milestone commit: `feat(core): local add/commit/verify implemented`.

### Phase 2 — Basic network & exchange

* Implement: libp2p bootstrap, `peer add`, direct manifest request/response, chunk request/response, `pull` & `share`.
* Tests: local multi‑process 3‑node integration.
* Milestone commit: `feat(net): basic discovery + manifest/chunk exchange`.

### Phase 3 — PubSub & parallel sync

* Implement: PubSub announcements, `sync`, parallel chunk downloads, resume.
* Tests: flaky network emulation, transfer benchmarks (100MB/1GB).
* Milestone commit: `feat(sync): pubsub announcements + parallel fetch`.

### Phase 4 — Security & provenance

* Implement: commit signing, key management, optional repo encryption, revocation.
* Tests: tamper simulation, signature verification.
* Milestone commit: `feat(security): signing + repo encryption`.

### Phase 5 — UX, packaging, docs

* Implement: man pages, shell completion, README, sample repos, installers.
* Milestone: `chore(release): docs + packaging`.

### Phase 6 — Beta & community mirrors

* Launch public beta, coordinate volunteer bootstrap nodes, collect bugs, add features.
* Milestone: `release(beta): public beta`.


## 6. Tests, verification & acceptance criteria

**Unit tests**

* Chunker edge cases: empty file, file smaller than chunk, boundary alignment, rolling chunk variations.
* Hash correctness: compare against reference vectors for Blake3 and optional SHA256 mode.
* Manifest canonicalization roundtrip.

**Integration tests**

* Multi‑node (3) in local containers: announce → sync → verify for 50MB/500MB/1GB artifacts.
* Resume tests: kill process mid‑transfer and resume.
* Corruption tests: flip data in stored chunk → verify fails.

**Performance tests**

* Throughput: measure wall clock time for 100MB, 1GB, 5GB transfers under varied parallelism.
* Memory profiling: ensure peak memory remains bounded (configurable concurrency cap).

**Security tests**

* Signature verification acceptance/rejection.
* Key rotation / revocation tests.

**Acceptance per phase**

* Phase 1: `add->commit->verify` succeeds with unit coverage ≥90%.
* Phase 2: 3‑node sync success for 100MB in CI emulation.
* Phase 3: 1GB transfers succeed in >80% runs under emulated NAT traversal.
* Phase 4: signatures verified; tampered manifests rejected.

---

## 7. Repo layout & commit discipline

```
shard/
├─ cmd/shard/             # CLI entry (main)
├─ core/                  # chunker, object store, manifest, commit DAG
├─ net/                   # libp2p adapters, pubsub, discovery
├─ crypto/                # key mgmt, signing, verification
├─ storage/               # index (sled/sqlite) adapters
├─ tests/                 # unit + integration + performance
├─ examples/              # sample artifact repos + scripts
├─ docs/                  # protocol.md, schemas, onboarding
├─ scripts/               # run-local-nodes.sh, stress tests
├─ build/                 # packaging helpers
├─ Cargo.toml
└─ README.md
```

**Commit strategy**

* Micro‑commits after each change.
* branch per feature.
* Use conventional commits: `feat(core): ...`, `fix(net): ...`, `test(integration): ...`.
* Tag releases semver style: `v0.1.0-alpha`.
* Phase milestone commits must include test results summary in commit message.

---

## 8. Automation & CI (no cloud cost mode)

* CI runs locally on developer machines or community self‑hosted runners.
* Provide `make dev-ci` which runs the test matrix locally (unit + integration with containerized nodes).
* Avoid cloud egress; use local artifacts for test files.

## Github

* Use github actions for CI.
* Use github releases for releases.
* Use github issues for bug reports.
* Use github pull requests for code changes.

## 9. Acceptance test (detailed script)

**Name:** 3‑node 1GB roundtrip

**Environment:** 3 containers on single host or 3 machines on LAN

1. Node A: `shard init; shard add big.bin; shard commit -m v0; shard share`
2. Node B: `shard sync` (auto discover)
3. Node C: `shard pull <A> <commit>`
4. On B & C: `shard verify <commit>`
5. Kill Node B mid‑transfer, restart and `shard sync` → transfer must resume.

**Pass criteria:** verify passes and checksums match original.



## 10. Security, privacy & governance

* Public repos: metadata signed and discoverable via pubsub.
* Private repos: optional encryption; peers must share symmetric key OOB.
---

## 11. Risks & mitigations

* **NAT traversal failure:** implement WebRTC + manual peer add + bootstrap list.
* **Bandwidth abuse:** rate limits, sponsor mirrors to relieve hot peers.
* **Key compromise:** key rotation via revocation commits and short key lifetimes for CI bots.
* **Adoption friction:** ship simple onboarding + examples for Hugging Face style repos.
