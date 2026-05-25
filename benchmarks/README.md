# Shard Benchmarks

Reproducible performance benchmarks for Shard operations.

## Quick Start

```bash
# Run default 1GB benchmark
./benchmark.sh

# Run 512MB benchmark
./benchmark.sh 512

# Run with JSON output (for automation)
./benchmark.sh 1024 json
```

## What This Measures

| Operation | Description |
|-----------|-------------|
| `shard add` | Chunking (4MiB fixed) + compression (zstd) + hashing (blake3) + local store |
| `shard commit` | Creating signed commit manifest |

## Example Results (1GB random data)

```
  Operation                  Time (seconds)
  ----------------------------------------
  shard add                            2.08
  shard commit                         0.02
  ----------------------------------------
  TOTAL (add + commit)                 2.10

  Local throughput: 487.62 MB/s
```

To run yourself:
```bash
./benchmark.sh 1024
```

## Scope

This benchmark measures **local** operations only. It does NOT measure:
- P2P network transfer (see `p2p_bench.sh`)
- Git LFS push times (includes network upload to remote server)

The chunking + compression throughput (~500 MB/s) is the core metric here. This is what dominates when adding large ML artifacts.

## Requirements

- `shard` CLI in PATH (or set `SHARD_BIN` environment variable)
- `python3` for nanosecond-precision timing