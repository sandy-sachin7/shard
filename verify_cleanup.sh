#!/bin/bash
set -e

SHARD_BIN=$(realpath target/debug/shard)
TEST_DIR=$(mktemp -d)
echo "Test dir: $TEST_DIR"

cd $TEST_DIR

# Node A
mkdir node_a
cd node_a
$SHARD_BIN init
echo "Hello P2P" > hello.txt
$SHARD_BIN add hello.txt
COMMIT_OUTPUT=$($SHARD_BIN commit -m "First commit")
COMMIT_ID=$(echo "$COMMIT_OUTPUT" | grep "Committed" | awk '{print $2}')
echo "Commit ID: $COMMIT_ID"

# Start sharing
$SHARD_BIN share > share.log 2>&1 &
SHARE_PID=$!
echo "Share PID: $SHARE_PID"

# Wait for node to start and get address
sleep 2
LISTEN_ADDR=$(grep "Listening on" share.log | head -n 1 | awk '{print $3}' | tr -d '"')
PEER_ID=$(grep "Local peer id:" share.log | head -n 1 | awk '{print $4}')
MULTIADDR="$LISTEN_ADDR/p2p/$PEER_ID"
echo "Multiaddr: $MULTIADDR"

if [ -z "$LISTEN_ADDR" ] || [ -z "$PEER_ID" ]; then
    echo "Failed to get multiaddr or peer id"
    cat share.log
    kill $SHARE_PID
    exit 1
fi

cd ..

# Node B
mkdir node_b
cd node_b
$SHARD_BIN init

# Add peer (optional, pull takes peer arg, but let's test peer add too)
$SHARD_BIN peer add "$MULTIADDR"

# Pull
$SHARD_BIN pull "$MULTIADDR" "$COMMIT_ID"

# Verify
if [ -f hello.txt ]; then
    CONTENT=$(cat hello.txt)
    if [ "$CONTENT" == "Hello P2P" ]; then
        echo "Verification SUCCESS!"
    else
        echo "Verification FAILED: Content mismatch. Got '$CONTENT'"
        exit 1
    fi
else
    echo "Verification FAILED: File not found"
    exit 1
fi

# Cleanup
kill $SHARE_PID
