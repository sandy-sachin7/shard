#!/bin/bash
# Shard Benchmark Script
# Reproducible performance testing for local ML artifact operations
#
# WHAT THIS MEASURES:
#   - shard add:     chunking + compression + local store throughput
#   - shard commit: metadata creation latency
#
# WHAT THIS DOES NOT MEASURE:
#   - Network transfer (P2P push/pull requires two nodes)
#   - Git LFS push times (includes network upload to remote)
#
# Usage: ./benchmark.sh [size_in_mb] [output_format]
#   size_in_mb: File size to test (default: 1024 = 1GB)
#   output_format: "table" (default) or "json"

SIZE_MB="${1:-1024}"
OUTPUT_FORMAT="${2:-table}"
SHARD_BIN="${SHARD_BIN:-shard}"
TMPDIR="$(mktemp -d)"
REPO_DIR=""

cleanup() {
    if [ -n "$REPO_DIR" ] && [ -d "$REPO_DIR/.shard" ]; then
        rm -rf "$REPO_DIR"
    fi
    rm -rf "$TMPDIR"
}
trap cleanup EXIT

log() { echo "[$(date +%H:%M:%S)] $*" >&2; }

now_ns() {
    python3 -c "import time; print(int(time.time_ns()))"
}

elapsed_ns() {
    local start=$1 end=$2
    python3 -c "print(f'{(($2 - $1) / 1000000000):.2f}')"
}

echo ""
echo "=============================================="
echo "  Shard Local Benchmark — $SIZE_MB MB file"
echo "=============================================="
echo ""
log "Generating ${SIZE_MB} MB random test file..."
TEST_FILE="$TMPDIR/test_model.bin"
dd if=/dev/urandom of="$TEST_FILE" bs=1M count=$SIZE_MB status=none 2>/dev/null
FILE_SIZE_BYTES=$(stat -c%s "$TEST_FILE")
FILE_SIZE_HUMAN="${FILE_SIZE_BYTES} bytes"
log "Generated: $FILE_SIZE_HUMAN"

echo ""
log "Creating temporary repository..."
REPO_DIR="$TMPDIR/test_repo"
mkdir -p "$REPO_DIR"
cd "$REPO_DIR"
$SHARD_BIN init --chunker fixed > /dev/null 2>&1

echo ""
log "Running benchmarks..."

START_ADD=$(now_ns)
$SHARD_BIN add "$TEST_FILE" > /dev/null 2>&1
END_ADD=$(now_ns)
ADD_TIME=$(elapsed_ns $START_ADD $END_ADD)

START_COMMIT=$(now_ns)
$SHARD_BIN commit -m "benchmark commit" --author "Benchmark <bench@shard.test>" > /dev/null 2>&1
END_COMMIT=$(now_ns)
COMMIT_TIME=$(elapsed_ns $START_COMMIT $END_COMMIT)

TOTAL_TIME=$(python3 -c "print(f'{$ADD_TIME + $COMMIT_TIME:.2f}')")
THROUGHPUT=$(python3 -c "print(f'{$SIZE_MB / float($TOTAL_TIME):.2f}')")

if [ "$OUTPUT_FORMAT" = "json" ]; then
    python3 -c "
import json
print(json.dumps({
    'operation': 'shard add',
    'size_mb': $SIZE_MB,
    'duration_seconds': $ADD_TIME,
    'throughput': f'{($SIZE_MB / float($ADD_TIME)):.2f} MB/s'
}))
print(json.dumps({
    'operation': 'shard commit',
    'size_mb': $SIZE_MB,
    'duration_seconds': $COMMIT_TIME
}))
print(json.dumps({
    'operation': 'total (add + commit)',
    'size_mb': $SIZE_MB,
    'duration_seconds': $TOTAL_TIME,
    'throughput': '${THROUGHPUT} MB/s'
}))
"
else
    echo ""
    echo "  Operation                  Time (seconds)"
    echo "  ----------------------------------------"
    printf "  %-28s %12s\n" "shard add" "$ADD_TIME"
    printf "  %-28s %12s\n" "shard commit" "$COMMIT_TIME"
    echo "  ----------------------------------------"
    printf "  %-28s %12s\n" "TOTAL (add + commit)" "$TOTAL_TIME"
    printf "\n  Local throughput: %s MB/s\n" "$THROUGHPUT"
    echo ""
    echo "=============================================="
    echo "  Scope & Limitations"
    echo "=============================================="
    echo ""
    echo "  This benchmark measures LOCAL operations only:"
    echo "    - Chunking (fixed 4MiB blocks)"
    echo "    - Compression (zstd)"
    echo "    - Blake3 hashing"
    echo "    - Local object storage"
    echo ""
    echo "  NOT measured (requires P2P setup):"
    echo "    - shard share:  announce to network"
    echo "    - shard pull:   P2P transfer speed"
    echo "    - shard push:   P2P push speed"
    echo ""
    echo "  The README claims ~40 sec for 10GB push (P2P)."
    echo "  Run benchmarks/p2p_bench.sh on two machines to verify."
fi

echo ""
log "Benchmark complete."