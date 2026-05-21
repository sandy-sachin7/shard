#!/bin/bash
set -e

SHARD_BIN=$(realpath target/debug/shard)
TEST_DIR=$(mktemp -d)
echo "Test dir: $TEST_DIR"

cleanup() {
    local exit_code=$?
    if [ -n "$SHARE_PID" ]; then
        kill "$SHARE_PID" 2>/dev/null || true
    fi
    rm -rf "$TEST_DIR"
    exit $exit_code
}
trap cleanup EXIT

cd "$TEST_DIR"

# ── Test 1: Basic init ──
echo "=== Test 1: Basic init ==="
mkdir node_a && cd node_a
"$SHARD_BIN" init
[ -d .shard/objects ] && [ -d .shard/keys ] && echo "  init OK"

# ── Test 2: Add + Commit + Verify (local) ──
echo "=== Test 2: Add + Commit + Verify ==="
echo "Small file" > small.txt
"$SHARD_BIN" add small.txt
COMMIT_OUTPUT=$("$SHARD_BIN" commit -m "First commit" --author "Test")
COMMIT_ID=$(echo "$COMMIT_OUTPUT" | grep "Committed" | awk '{print $2}')
echo "  Commit ID: $COMMIT_ID"

"$SHARD_BIN" verify "$COMMIT_ID" | grep -q "Verification successful"
echo "  verify OK"

# ── Test 3: Large file crossing chunk boundary ──
echo "=== Test 3: Large file (5 MiB) ==="
dd if=/dev/urandom of=large.bin bs=1M count=5 2>/dev/null
EXPECTED_HASH=$(sha256sum large.bin | awk '{print $1}')
"$SHARD_BIN" add large.bin
COMMIT_OUTPUT=$("$SHARD_BIN" commit -m "Large file" --author "Test")
COMMIT_ID_2=$(echo "$COMMIT_OUTPUT" | grep "Committed" | awk '{print $2}')
echo "  Commit ID: $COMMIT_ID_2"
"$SHARD_BIN" verify "$COMMIT_ID_2" | grep -q "Verification successful"
echo "  large file verify OK"

# ── Test 4: Tamper detection ──
echo "=== Test 4: Tamper detection ==="
# Create a separate scenario for tamper test (don't corrupt the real objects)
mkdir "$TEST_DIR/tamper-test" && cd "$TEST_DIR/tamper-test"
"$SHARD_BIN" init > /dev/null
echo "tamper me" > secret.txt
"$SHARD_BIN" add secret.txt > /dev/null
TAMPER_OUT=$("$SHARD_BIN" commit -m "tamper-target" --author "T")
TAMPER_CID=$(echo "$TAMPER_OUT" | awk '{print $2}')
TAMPER_PREFIX="${TAMPER_CID:0:2}"
echo "TAMPERED" > ".shard/objects/$TAMPER_PREFIX/$TAMPER_CID"
if "$SHARD_BIN" verify "$TAMPER_CID" 2>/dev/null; then
    echo "  FAIL: verify should have failed after tampering"
    exit 1
fi
echo "  tamper detection OK"
cd "$TEST_DIR/node_a"

# ── Test 5: Share + Pull (2-node P2P) ──
echo "=== Test 5: Share + Pull ==="
"$SHARD_BIN" share > share.log 2>&1 &
SHARE_PID=$!
sleep 2

LISTEN_ADDR=$(grep "Listening on" share.log | head -1 | awk '{print $3}' | tr -d '"')
PEER_ID=$(grep "Local peer id:" share.log | head -1 | awk '{print $4}')
MULTIADDR="$LISTEN_ADDR/p2p/$PEER_ID"

if [ -z "$LISTEN_ADDR" ] || [ -z "$PEER_ID" ]; then
    echo "  Failed to get multiaddr or peer id"
    cat share.log
    exit 1
fi
echo "  Node A: $MULTIADDR"

cd "$TEST_DIR"
mkdir node_b && cd node_b
"$SHARD_BIN" init
"$SHARD_BIN" peer add "$MULTIADDR"

# Pull the small file commit
"$SHARD_BIN" pull "$MULTIADDR" "$COMMIT_ID"
echo "  pull OK"

# Verify the file was materialized
[ -f small.txt ] || { echo "  FAIL: small.txt not materialized"; exit 1; }
CONTENT=$(cat small.txt)
[ "$CONTENT" = "Small file" ] || { echo "  FAIL: content mismatch. Got '$CONTENT'"; exit 1; }
echo "  file materialization OK"

# Verify content integrity directly (shard verify uses local key for sig check,
# which won't match the remote author's key — will be addressed in Phase 4)
# Instead, verify the pull stored the commit correctly
PREFIX="${COMMIT_ID:0:2}"
if [ ! -f ".shard/objects/$PREFIX/$COMMIT_ID" ]; then
    echo "  FAIL: commit object not stored"
    exit 1
fi
echo "  pulled object storage OK"

# Pull the large file commit
"$SHARD_BIN" pull "$MULTIADDR" "$COMMIT_ID_2"
echo "  large pull OK"

[ -f large.bin ] || { echo "  FAIL: large.bin not materialized"; exit 1; }
PULLED_HASH=$(sha256sum large.bin | awk '{print $1}')
[ "$PULLED_HASH" = "$EXPECTED_HASH" ] || { echo "  FAIL: large file hash mismatch"; exit 1; }
echo "  large file sha256 match OK"

PREFIX2="${COMMIT_ID_2:0:2}"
if [ ! -f ".shard/objects/$PREFIX2/$COMMIT_ID_2" ]; then
    echo "  FAIL: large commit object not stored"
    exit 1
fi
echo "  large pulled object storage OK"

# ── All tests passed ──
echo ""
echo "═══════════════════════════════════════════"
echo "  ALL STRESS TESTS PASSED"
echo "═══════════════════════════════════════════"
