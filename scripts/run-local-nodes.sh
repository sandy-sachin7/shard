#!/usr/bin/env bash
set -euo pipefail

# run-local-nodes.sh — Start a local 3-node shard P2P test cluster
#
# Usage: ./scripts/run-local-nodes.sh [workdir]
#
# Starts three shard nodes in separate terminals or background processes:
#   Node A: relay server on /ip4/127.0.0.1/tcp/9900
#   Node B: peer connecting via relay
#   Node C: peer connecting via relay
#
# Requires: shard to be installed (cargo install --path cmd/shard-cli)

WORKDIR="${1:-/tmp/shard-cluster}"
mkdir -p "$WORKDIR"

echo "=== shard local 3-node cluster ==="
echo "Workdir: $WORKDIR"
echo ""

# Clean up on exit
cleanup() {
    echo "Shutting down nodes..."
    for pid_file in "$WORKDIR"/node_*.pid; do
        if [ -f "$pid_file" ]; then
            kill "$(cat "$pid_file")" 2>/dev/null || true
            rm -f "$pid_file"
        fi
    done
    exit 0
}
trap cleanup SIGINT SIGTERM EXIT

# Node A: Relay server
NODE_A_DIR="$WORKDIR/node_a"
mkdir -p "$NODE_A_DIR"
echo "Starting Node A (relay server) in $NODE_A_DIR"
(cd "$NODE_A_DIR" && shard init --db flat 2>/dev/null && shard relay --listen "/ip4/127.0.0.1/tcp/9900") &
NODE_A_PID=$!
echo $NODE_A_PID > "$WORKDIR/node_a.pid"
echo "  PID: $NODE_A_PID"
sleep 2

# Node B: Regular peer
NODE_B_DIR="$WORKDIR/node_b"
mkdir -p "$NODE_B_DIR"
echo "Starting Node B in $NODE_B_DIR"
(cd "$NODE_B_DIR" && shard init --db flat 2>/dev/null && echo "node_b_ready" > "$WORKDIR/node_b.ready" && shard share) &
NODE_B_PID=$!
echo $NODE_B_PID > "$WORKDIR/node_b.pid"
echo "  PID: $NODE_B_PID"
sleep 1

# Node C: Regular peer
NODE_C_DIR="$WORKDIR/node_c"
mkdir -p "$NODE_C_DIR"
echo "Starting Node C in $NODE_C_DIR"
(cd "$NODE_C_DIR" && shard init --db flat 2>/dev/null && echo "node_c_ready" > "$WORKDIR/node_c.ready" && shard share) &
NODE_C_PID=$!
echo $NODE_C_PID > "$WORKDIR/node_c.pid"
echo "  PID: $NODE_C_PID"

echo ""
echo "All 3 nodes started. Press Ctrl+C to stop."
echo ""

# Wait for any child to exit
wait
