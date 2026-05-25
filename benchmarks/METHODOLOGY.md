# Benchmark Methodology

## Local Benchmarks (`benchmark.sh`)

### What is measured

| Operation | What it does |
|-----------|--------------|
| `shard add` | Read file → chunk (4MiB fixed) → compress (zstd) → hash (blake3) → store in `.shard/objects/` |
| `shard commit` | Create signed commit manifest, update HEAD |

### What is NOT measured

- **Network transfer**: `shard add` and `shard commit` are purely local
- **P2P operations**: `share`, `pull`, `push` require two nodes
- **Git LFS push**: Includes upload to remote server (network-bound)

### Why local benchmarks matter

Chunking + compression + hashing is the core of Shard's performance advantage:
- **Fixed 4MiB chunks** → predictable I/O patterns, cache-friendly
- **Zstd compression** → 3-5x ratio reduces disk I/O
- **Blake3 hashing** → SIMD-accelerated, ~1 GB/s

These are the operations that dominate real workloads. A 10GB model checkpoint spend most of its time in chunking/compression, not network transfer.

### Extrapolation to 10GB

```bash
# 1GB benchmark result
./benchmark.sh 1024
# e.g., TOTAL = 1.2 seconds → throughput = 833 MB/s

# Linear extrapolation to 10GB (approximate)
echo "1.2 * 10 = 12 seconds"
```

**Note**: Linear extrapolation is approximate. At larger sizes, RAM cache effects diminish and real performance may be slightly lower.

---

## P2P Benchmarks (`p2p_bench.sh`)

To measure actual network transfer speeds:

```bash
# Node A (receiver)
./benchmarks/p2p_bench.sh receive

# Node B (sender) — after Node A shows its listen address
./benchmarks/p2p_bench.sh send <node_a_multiaddr> 1024
```

This measures `shard pull` — the actual P2P transfer with chunking, compression, and network I/O.

---

## Git LFS Comparison

The README comparison table shows:

| Operation | Git LFS | Shard |
|-----------|---------|-------|
| Initial push (10GB) | ~5 min | ~40 sec |

### Source of Git LFS number

Git LFS push time depends on:
- Network bandwidth to LFS server (typically 50-100 Mbps for cloud VMs)
- File size (10 GB = 80 Gbits)
- Server-side processing

**Calculation**: 80 Gbits / 75 Mbps ≈ 1067 seconds ≈ 18 minutes (conservative)
**Reported**: ~5 min (optimistic, on fast connection)

### Source of Shard number

P2P transfer of 10GB with:
- Rabin CDC chunking (dedup ratio depends on data similarity)
- Zstd compression (3-5x ratio)
- Direct peer-to-peer (no server bottleneck)

**Estimated**: ~40 seconds on 1 Gbps peer connection

### Why these are hard to compare

- Git LFS requires a Git LFS server (centralized)
- Shard P2P requires a connected peer (decentralized)
- Network conditions vary wildly

---

## Reproducibility Checklist

To verify benchmark results:

1. Run `./benchmark.sh 1024` locally — confirm throughput
2. For P2P: run `p2p_bench.sh` on two machines on same LAN
3. For Git LFS: set up a Git LFS server and measure actual push time

Results are architecture, OS, and hardware dependent. Report:
- CPU model
- RAM size
- Disk type (SSD vs HDD)
- Network setup