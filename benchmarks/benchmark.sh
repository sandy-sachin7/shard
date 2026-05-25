#!/bin/bash
# Shard Benchmark Script
# Reproducible performance testing for ML artifact operations
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

elapsed_ns() {
    local start=$1 end=$2
    echo "$start $end" | awk '{printf "%.2f", ($2 - $1) / 1000000000}'
}

echo ""
echo "=============================================="
echo "  Shard Benchmark — $SIZE_MB MB file"
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

START_ADD=$(date +%s%N)
$SHARD_BIN add "$TEST_FILE" > /dev/null 2>&1
END_ADD=$(date +%s%N)
ADD_TIME=$(elapsed_ns $START_ADD $END_ADD)

START_COMMIT=$(date +%s%N)
$SHARD_BIN commit -m "benchmark commit" --author "Benchmark <bench@shard.test>" > /dev/null 2>&1
END_COMMIT=$(date +%s%N)
COMMIT_TIME=$(elapsed_ns $START_COMMIT $END_COMMIT)

TOTAL_TIME=$(echo "$ADD_TIME $COMMIT_TIME" | awk '{printf "%.2f", $1 + $2}')
THROUGHPUT=$(echo "$SIZE_MB $TOTAL_TIME" | awk '{printf "%.2f", $1 / $2}')
EXTRAPOLATED=$(echo "$TOTAL_TIME" | awk '{printf "%.2f", $1 * 10}')

if [ "$OUTPUT_FORMAT" = "json" ]; then
    cat <<EOF
{"operation": "shard add", "size_mb": $SIZE_MB, "duration_seconds": $ADD_TIME, "throughput": "$(echo "$SIZE_MB $ADD_TIME" | awk '{printf "%.2f MB/s", $1 / $2}')"}
{"operation": "shard commit", "size_mb": $SIZE_MB, "duration_seconds": $COMMIT_TIME}
{"operation": "total (add + commit)", "size_mb": $SIZE_MB, "duration_seconds": $TOTAL_TIME, "throughput": "${THROUGHPUT} MB/s"}
EOF
else
    echo ""
    echo "  Operation                  Time (seconds)"
    echo "  ----------------------------------------"
    printf "  %-28s %12.2f\n" "shard add" "$ADD_TIME"
    printf "  %-28s %12.2f\n" "shard commit" "$COMMIT_TIME"
    echo "  ----------------------------------------"
    printf "  %-28s %12.2f\n" "TOTAL (add + commit)" "$TOTAL_TIME"
    printf "\n  %-28s %12s MB/s\n" "Throughput (add + commit)" "$THROUGHPUT"
    echo ""
    echo "=============================================="
    echo "  Extrapolation to 10GB"
    echo "=============================================="
    echo ""
    echo "  Projected time for 10GB: ~${EXTRAPOLATED} seconds"
    echo ""
    echo "  Comparison (from README):"
    echo "    Git LFS 10GB push: ~300 seconds (5 min)"
    echo "    Shard 10GB push:   ~${EXTRAPOLATED} seconds"
    echo ""
    echo "  NOTE: Git LFS benchmark not included — run manually with:"
    echo "    git lfs track '*.bin'"
    echo "    cp $TEST_FILE test_model.bin"
    echo "    git add test_model.bin"
    echo "    time git commit -m 'bench'"
fi

echo ""
log "Benchmark complete."