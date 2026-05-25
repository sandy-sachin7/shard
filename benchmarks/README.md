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
| `shard add` | Chunking + hashing + compression + storing a file |
| `shard commit` | Creating signed commit manifest |

## Extrapolating to 10GB

```bash
# Run 1GB benchmark
./benchmark.sh 1024

# Multiply the TOTAL by 10 to get projected 10GB time
# e.g., if TOTAL = 4 sec → projected 10GB ≈ 40 sec
```

## Comparing with Git LFS

```bash
# Setup
git lfs install
git lfs track "*.bin"

# Time a 1GB push
dd if=/dev/urandom of=test.bin bs=1M count=1024
git add test.bin
time git commit -m "bench"

# Multiply by 10 for 10GB comparison
```

## Requirements

- `shard` CLI in PATH (or set `SHARD_BIN` environment variable)
- `bc` for floating-point arithmetic
- `numfmt` for human-readable file sizes