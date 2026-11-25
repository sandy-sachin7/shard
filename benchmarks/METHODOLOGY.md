# Benchmark Methodology

All performance numbers reported in Shard's `README.md` are **directly measured** using reproducible scripts in this directory — no estimated or extrapolated network tables.

## Local End-to-End Benchmarks (`bench_real.py`)

Run with:

```bash
cargo build --release
python3 benchmarks/bench_real.py --size-mb 256
```

### Workloads

1. **High-entropy binary weights (`weights.bin`)**: Independent `os.urandom` bytes (`1.00×` Zstd compressibility), representative of trained FP16/BF16/FP32 neural network weights.
   - `weights_append_5pct`: appends 5% new bytes to the end of the file.
   - `weights_prepend_1mb`: prepends 1 MiB at byte 0 (shifting every subsequent offset — worst case for fixed chunking, ideal test for Rabin CDC).
2. **Structured JSONL dataset (`dataset.jsonl`)**: Unique training records with realistic text + token arrays.
   - `jsonl_prepend_1mb`: prepends 1 MiB of new records at the start of the dataset.

### Measured Metrics

| Operation | What it measures |
| :--- | :--- |
| `shard add` + `shard commit` (v1) | Initial chunking (`fixed` 4 MiB or `rabin` CDC) + Zstd compression + Blake3 hashing + Ed25519 signing + `.shard/objects` size |
| `shard add` + `shard commit` (v2) | Incremental chunking + exact delta bytes added to `.shard/objects` (deduplication savings) |
| `shard verify` | Full Blake3 content hash, Merkle root, and Ed25519 signature verification |
| `shard checkout` | Full artifact reconstruction from `.shard/objects` |

### Why Compressibility Depends on Artifact Class

- **Trained model weights (FP16/BF16/FP32)**: High entropy; Zstd compression ratio is typically `~1.00–1.15×`. Storage savings come from **chunk-level deduplication** across unchanged tensors/blocks, not compression.
- **Datasets (JSONL, CSV, text, uncompressed tarballs)**: Low entropy; Zstd typically achieves `3–10×` compression on top of Rabin CDC deduplication.

## P2P Transfer Benchmarks (`p2p_bench.sh`)

Network transfer speed depends heavily on link bandwidth, NAT/relay topology, and disk I/O. Measure real P2P transfers between two machines on your network with:

```bash
# Node A (receiver)
./benchmarks/p2p_bench.sh receive

# Node B (sender) — after Node A prints its multiaddr
./benchmarks/p2p_bench.sh send <node_a_multiaddr> 1024
```

## Criterion Micro-Benchmarks

```bash
cargo bench -p shard-core
```

## Reporting Results

When sharing benchmark numbers in issues or PRs, include CPU model, RAM size, OS/kernel, disk type (NVMe/SSD/HDD), and `shard --version`.
