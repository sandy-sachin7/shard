#!/bin/bash
# Shard P2P Benchmark Script
# Measures actual peer-to-peer transfer speed between two nodes
#
# Usage:
#   Node A (receiver): ./p2p_bench.sh receive [port]
#   Node B (sender):   ./p2p_bench.sh send <node_a_multiaddr> [size_mb]
#
# Example:
#   Node A: ./p2p_bench.sh receive 9000
#   Node B: ./p2p_bench.sh send /ip4/192.168.1.10/tcp/9000/p2p/Qm... 1024

set -e

MODE="${1:-}"
PEER_ADDR="${2:-}"
SIZE_MB="${3:-1024}"
PORT="${PORT:-9000}"
SHARD_BIN="${SHARD_BIN:-shard}"
TMPDIR="$(mktemp -d)"

cleanup() {
    rm -rf "$TMPDIR"
}
trap cleanup EXIT

log() { echo "[$(date +%H:%M:%S)] $*" >&2; }

now_ns() {
    python3 -c "import time; print(int(time.time_ns()))"
}

elapsed_ns() {
    python3 -c "print(f'{(($2 - $1) / 1000000000):.2f}')"
}

init_repo() {
    mkdir -p "$TMPDIR/repo"
    cd "$TMPDIR/repo"
    $SHARD_BIN init --chunker fixed > /dev/null 2>&1
}

wait_for_file() {
    local file="$1"
    local timeout="${2:-60}"
    local count=0
    while [ ! -f "$file" ] && [ $count -lt $timeout ]; do
        sleep 1
        count=$((count + 1))
    done
    if [ ! -f "$file" ]; then
        echo "ERROR: Timeout waiting for $file" >&2
        return 1
    fi
    return 0
}

case "$MODE" in
    receive)
        echo "=============================================="
        echo "  Shard P2P Benchmark — Receiver Node"
        echo "=============================================="
        echo ""
        log "Initializing repository..."
        init_repo

        log "Generating ${SIZE_MB} MB test file..."
        TEST_FILE="$TMPDIR/test_model.bin"
        dd if=/dev/urandom of="$TEST_FILE" bs=1M count=$SIZE_MB status=none 2>/dev/null

        log "Adding and committing test file..."
        $SHARD_BIN add "$TEST_FILE" > /dev/null 2>&1
        COMMIT_ID=$($SHARD_BIN commit -m "benchmark" --author "Bench <bench@shard.test>" 2>&1 | grep -oE '[a-f0-9]{64}' | head -1)

        echo ""
        log "Starting P2P listener on port $PORT..."
        log "Run on peer: $SHARD_BIN pull /ip4/<this-ip>/tcp/$PORT <commit_id>"
        log "Waiting for incoming transfer..."

        START_TIME=$(now_ns)

        timeout 300 $SHARD_BIN share > /dev/null 2>&1 &
        SHARE_PID=$!

        sleep 2

        timeout 300 $SHARD_BIN sync 2>&1 | head -20 || true

        wait $SHARE_PID 2>/dev/null || true

        END_TIME=$(now_ns)
        ELAPSED=$(elapsed_ns $START_TIME $END_TIME)
        THROUGHPUT=$(python3 -c "print(f'{$SIZE_MB / float($ELAPSED):.2f}')")

        echo ""
        echo "  Transfer completed in ${ELAPSED} seconds"
        echo "  Throughput: ${THROUGHPUT} MB/s"
        ;;

    send)
        if [ -z "$PEER_ADDR" ]; then
            echo "Usage: $0 send <peer_multiaddr> [size_mb]"
            echo "  peer_multiaddr: Multiaddr of receiving node (from 'receive' mode)"
            exit 1
        fi

        echo "=============================================="
        echo "  Shard P2P Benchmark — Sender Node"
        echo "=============================================="
        echo ""
        log "Initializing repository..."
        init_repo

        log "Generating ${SIZE_MB} MB test file..."
        TEST_FILE="$TMPDIR/test_model.bin"
        dd if=/dev/urandom of="$TEST_FILE" bs=1M count=$SIZE_MB status=none 2>/dev/null

        log "Adding and committing test file..."
        $SHARD_BIN add "$TEST_FILE" > /dev/null 2>&1
        $SHARD_BIN commit -m "benchmark" --author "Bench <bench@shard.test>" > /dev/null 2>&1

        log "Connecting to peer: $PEER_ADDR"
        $SHARD_BIN peer add "$PEER_ADDR" > /dev/null 2>&1 || true

        echo ""
        log "Pushing to peer..."

        START_TIME=$(now_ns)
        $SHARD_BIN push "$PEER_ADDR" > /dev/null 2>&1
        END_TIME=$(now_ns)

        ELAPSED=$(elapsed_ns $START_TIME $END_TIME)
        THROUGHPUT=$(python3 -c "print(f'{$SIZE_MB / float($ELAPSED):.2f}')")

        echo ""
        echo "  Transfer completed in ${ELAPSED} seconds"
        echo "  Throughput: ${THROUGHPUT} MB/s"
        ;;

    *)
        echo "Shard P2P Benchmark"
        echo ""
        echo "Usage:"
        echo "  # Terminal 1 (receiver)"
        echo "  ./p2p_bench.sh receive [port]"
        echo ""
        echo "  # Terminal 2 (sender)"
        echo "  ./p2p_bench.sh send <peer_multiaddr> [size_mb]"
        echo ""
        echo "Requirements:"
        echo "  - Both nodes must have shard installed"
        echo "  - Network connectivity between nodes (firewall ports open)"
        echo "  - For same-machine testing, use different ports"
        exit 1
        ;;
esac