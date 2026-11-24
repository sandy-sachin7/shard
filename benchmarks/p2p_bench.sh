#!/bin/bash
# Shard P2P Benchmark Script
# Measures actual peer-to-peer transfer speed between two nodes
#
# Usage:
#   Node A (receiver): ./p2p_bench.sh receive [port] [sender_pubkey_hex]
#   Node B (sender):   ./p2p_bench.sh send <node_a_multiaddr_with_p2p> [size_mb]
#
# Example:
#   Node A: ./p2p_bench.sh receive 9000
#   Node B: ./p2p_bench.sh send /ip4/192.168.1.10/tcp/9000/p2p/12D3KooW... 1024

set -euo pipefail

MODE="${1:-}"
ARG2="${2:-}"
ARG3="${3:-}"
SHARD_BIN="${SHARD_BIN:-shard}"
TMPDIR="$(mktemp -d)"

cleanup() {
    if [ -n "${SHARE_PID:-}" ] && kill -0 "$SHARE_PID" 2>/dev/null; then
        kill "$SHARE_PID" 2>/dev/null || true
    fi
    rm -rf "$TMPDIR"
}
trap cleanup EXIT

log() { echo "[$(date +%H:%M:%S)] $*" >&2; }

now_ns() {
    python3 -c "import time; print(int(time.time_ns()))"
}

elapsed_s() {
    python3 -c "print(f'{max(($2 - $1) / 1_000_000_000, 0.001):.2f}')"
}

init_repo() {
    mkdir -p "$TMPDIR/repo"
    cd "$TMPDIR/repo"
    "$SHARD_BIN" init --chunker fixed > /dev/null 2>&1
}

case "$MODE" in
    receive)
        PORT="${ARG2:-9000}"
        SENDER_PUBKEY="${ARG3:-}"

        echo "=============================================="
        echo "  Shard P2P Benchmark — Receiver Node"
        echo "=============================================="
        echo ""
        log "Initializing receiver repository..."
        init_repo

        if [ -n "$SENDER_PUBKEY" ]; then
            log "Authorizing sender public key..."
            "$SHARD_BIN" key add-authorized "$SENDER_PUBKEY" > /dev/null
        else
            log "NOTE: For push benchmarks, pass the sender's 64-hex public key as arg 3"
            log "      or run 'shard key add-authorized <pubkey_hex>' in $TMPDIR/repo."
        fi

        LISTEN_ADDR="/ip4/0.0.0.0/tcp/${PORT}"
        SHARE_LOG="$TMPDIR/share.log"
        log "Starting P2P listener on ${LISTEN_ADDR}..."
        "$SHARD_BIN" share --listen "$LISTEN_ADDR" > "$SHARE_LOG" 2>&1 &
        SHARE_PID=$!

        # Wait briefly for Peer ID to be printed
        PEER_ID=""
        for _ in $(seq 1 30); do
            if [ -f "$SHARE_LOG" ]; then
                PEER_ID=$(grep -oE '12D3KooW[1-9A-HJ-NP-Za-km-z]+' "$SHARE_LOG" | head -1 || true)
                if [ -n "$PEER_ID" ]; then
                    break
                fi
            fi
            sleep 0.2
        done

        echo ""
        if [ -n "$PEER_ID" ]; then
            log "Receiver Peer ID: $PEER_ID"
            log "Run on sender (pull/push multiaddr):"
            echo "  ./benchmarks/p2p_bench.sh send /ip4/<this-ip>/tcp/${PORT}/p2p/${PEER_ID} 1024"
        else
            log "Listening on ${LISTEN_ADDR} (check $SHARE_LOG for Peer ID)"
        fi
        log "Receiver is ready (Ctrl-C to stop)..."
        wait "$SHARE_PID" || true
        ;;

    send)
        PEER_ADDR="$ARG2"
        SIZE_MB="${ARG3:-1024}"

        if [ -z "$PEER_ADDR" ] || [[ "$PEER_ADDR" != */p2p/* ]]; then
            echo "Usage: $0 send <peer_multiaddr_with_p2p> [size_mb]"
            echo "  Example: $0 send /ip4/192.168.1.10/tcp/9000/p2p/12D3KooW... 1024"
            exit 1
        fi

        echo "=============================================="
        echo "  Shard P2P Benchmark — Sender Node"
        echo "=============================================="
        echo ""
        log "Initializing sender repository..."
        init_repo

        PUBKEY_HEX=$(python3 -c "print(open('.shard/keys/public.key','rb').read().hex())")
        log "Sender public key (authorize on receiver): $PUBKEY_HEX"

        log "Generating ${SIZE_MB} MB test file..."
        TEST_FILE="$TMPDIR/repo/test_model.bin"
        dd if=/dev/urandom of="$TEST_FILE" bs=1M count="$SIZE_MB" status=none

        log "Adding and committing test file..."
        "$SHARD_BIN" add "$TEST_FILE" > /dev/null
        COMMIT_ID=$("$SHARD_BIN" commit -m "benchmark" --author "Bench <bench@shard.test>" --json | python3 -c "import json,sys; print(json.load(sys.stdin)['commit_id'])")
        log "Committed $COMMIT_ID"

        log "Pushing to peer: $PEER_ADDR"
        START_TIME=$(now_ns)
        "$SHARD_BIN" push "$PEER_ADDR" > /dev/null
        END_TIME=$(now_ns)

        ELAPSED=$(elapsed_s "$START_TIME" "$END_TIME")
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
        echo "  ./benchmarks/p2p_bench.sh receive [port] [sender_pubkey_hex]"
        echo ""
        echo "  # Terminal 2 (sender)"
        echo "  ./benchmarks/p2p_bench.sh send /ip4/<receiver-ip>/tcp/<port>/p2p/<peer_id> [size_mb]"
        echo ""
        echo "Requirements:"
        echo "  - Both nodes must have shard installed"
        echo "  - Receiver must authorize the sender's public key before push"
        exit 1
        ;;
esac
