#!/bin/bash
# Comprehensive Shard Stress Test & Benchmark Suite
set -euo pipefail

SHARD_BIN="$(realpath target/release/shard)"
RESULTS_DIR="$(mktemp -d)"
MODEL_DIR="$RESULTS_DIR/models"
SCENARIO_DIR="$RESULTS_DIR/scenarios"
BIN_DIR="$RESULTS_DIR/bin"
mkdir -p "$MODEL_DIR" "$SCENARIO_DIR" "$BIN_DIR"
ln -sf "$SHARD_BIN" "$BIN_DIR/shard"
PATH="$BIN_DIR:$PATH"
export PATH

echo "{" > "$RESULTS_DIR/metrics.json"
echo '"timestamp": "'$(date -Iseconds)'",' >> "$RESULTS_DIR/metrics.json"
echo '"shard_bin": "'$SHARD_BIN'",' >> "$RESULTS_DIR/metrics.json"
FIRST=true

collect_metric() {
    local key="$1"
    local value="$2"
    local unit="$3"
    if [ "$FIRST" = true ]; then
        FIRST=false
    else
        echo "," >> "$RESULTS_DIR/metrics.json"
    fi
    echo -n "\"${key}\": {\"value\": ${value}, \"unit\": \"${unit}\"}" >> "$RESULTS_DIR/metrics.json"
}

cleanup_repo() {
    local dir="$1"
    rm -rf "$dir"
    mkdir -p "$dir"
}

bench() {
    local start=$(date +%s%N)
    "$@" >/dev/null 2>&1
    local end=$(date +%s%N)
    # Return seconds as a decimal (nanoseconds / 1e9)
    awk "BEGIN { printf \"%.9f\", ($end - $start) / 1000000000 }"
}

secs_to_ms() {
    awk "BEGIN { printf \"%.2f\", $1 * 1000 }"
}

# ──────────────────────────────────────────────
# Phase 1: Create Model Files
# ──────────────────────────────────────────────
echo "=== Phase 1: Creating model files ==="

# 1a: Happy path models
dd if=/dev/urandom of="$MODEL_DIR/tiny.dat" bs=1K count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/small.dat" bs=1K count=64 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/medium.dat" bs=1M count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/large.dat" bs=4M count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/xlarge.dat" bs=4M count=2 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/xxlarge.dat" bs=4M count=4 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/huge.dat" bs=16M count=1 2>/dev/null

# 1b: Edge-case models
dd if=/dev/zero  of="$MODEL_DIR/zero_file.dat" bs=1M count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/exact_chunk.dat" bs=4M count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/cross_chunk.dat" bs=4M count=1 2>/dev/null; dd if=/dev/urandom bs=1 count=1 >> "$MODEL_DIR/cross_chunk.dat" 2>/dev/null
truncate -s 0 "$MODEL_DIR/empty.dat"
echo -n "a" > "$MODEL_DIR/one_byte.dat"

# 1c: Many small files
mkdir -p "$MODEL_DIR/many_small"
for i in $(seq 1 100); do
    dd if=/dev/urandom of="$MODEL_DIR/many_small/file_${i}.dat" bs=1K count=1 2>/dev/null
done

# 1d: Large file for throughput stress
dd if=/dev/urandom of="$MODEL_DIR/stress_100m.dat" bs=1M count=100 2>/dev/null
echo "  Done creating model files"

# Record file sizes
echo "," >> "$RESULTS_DIR/metrics.json"
echo "\"model_files\": {" >> "$RESULTS_DIR/metrics.json"
FIRST_FILE=true
for f in "$MODEL_DIR"/*.dat "$MODEL_DIR"/many_small/*.dat; do
    [ -f "$f" ] || continue
    bname=$(basename "$f")
    size=$(stat --format=%s "$f")
    if [ "$FIRST_FILE" = true ]; then FIRST_FILE=false; else echo "," >> "$RESULTS_DIR/metrics.json"; fi
    hash=$(sha256sum "$f" | cut -d' ' -f1)
    echo -n "\"${bname}\": {\"bytes\": ${size}, \"sha256\": \"${hash}\"}" >> "$RESULTS_DIR/metrics.json"
done
echo "}" >> "$RESULTS_DIR/metrics.json"

# ──────────────────────────────────────────────
# Phase 2: Functional Scenario Tests
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 2: Functional scenario tests ==="

run_scenario() {
    local name="$1"
    local desc="$2"
    local expected_success="$3"
    local dir="$SCENARIO_DIR/$name"
    cleanup_repo "$dir"
    cd "$dir"

    local ok=true
    local output
    output=$(bash -c "cd '$dir' && $4" 2>&1) || ok=false

    local result="pass"
    if { [ "$expected_success" = "yes" ] && [ "$ok" = false ]; } || { [ "$expected_success" = "no" ] && [ "$ok" = true ]; }; then
        result="fail"
    fi
    echo "  [$result] $name: $desc"
    collect_metric "scenario.$name" "passed" "$( [ "$result" = "pass" ] && echo 1 || echo 0 )" "bool"
}

# HAPPY PATH
run_scenario "init_basic" "Initialize empty repo" "yes" "shard init"
run_scenario "add_small" "Add a small file" "yes" "shard init && cp $MODEL_DIR/small.dat . && shard add small.dat"
run_scenario "add_commit_verify" "Full cycle" "yes" "shard init && cp $MODEL_DIR/tiny.dat . && shard add tiny.dat && shard commit -m 'tiny' --author 'T' && CID=\$(shard log | grep '^commit ' | head -1 | awk '{print \$2}') && shard verify \$CID"
run_scenario "multi_file_commit" "Multiple files" "yes" "shard init && cp $MODEL_DIR/tiny.dat $MODEL_DIR/small.dat . && shard add tiny.dat && shard add small.dat && shard commit -m 'multi' --author 'T'"
run_scenario "large_file_commit" "Cross chunk boundary" "yes" "shard init && cp $MODEL_DIR/xlarge.dat . && shard add xlarge.dat && shard commit -m 'xlarge' --author 'T'"
run_scenario "checkout_restore" "Checkout restores" "yes" "shard init && echo 'hello' > restore.txt && shard add restore.txt && shard commit -m 'restore' --author 'T' && rm restore.txt && CID=\$(shard log | grep '^commit ' | head -1 | awk '{print \$2}') && shard checkout \$CID"
run_scenario "status_flow" "Status states" "yes" "shard init && shard status | grep -q 'No commits' && echo 'data' > f.txt && shard add f.txt && shard status | grep -q 'to be committed'"
run_scenario "tag_add_list" "Tag add/list" "yes" "shard init && echo 'x' > x.txt && shard add x.txt && shard commit -m 'tag-me' --author 'T' && CID=\$(shard log | grep '^commit ' | head -1 | awk '{print \$2}') && shard tag add v1 \$CID && shard tag list | grep -q v1"
run_scenario "config_set_get" "Config" "yes" "shard init && shard config set user.name 'Alice' && shard config get user.name | grep -q Alice"
run_scenario "prune_orphans" "Prune" "yes" "shard init && cp $MODEL_DIR/tiny.dat . && shard add tiny.dat && shard commit -m 'keep' --author 'T' && mkdir -p .shard/objects/ff && echo ORPHAN > .shard/objects/ff/ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff && shard prune | grep -q Pruned"

# NON-HAPPY PATH
run_scenario "init_twice_fails" "Cannot init twice" "no" "shard init && shard init"
run_scenario "verify_bad_commit" "Bad commit fails" "no" "shard init && shard verify 0000000000000000000000000000000000000000000000000000000000000000"
run_scenario "verify_tampered" "Tampered object fails" "no" "shard init && echo 'data' > secret.txt && shard add secret.txt && shard commit -m 'secret' --author 'T' && CID=\$(shard log | grep '^commit ' | head -1 | awk '{print \$2}') && echo TAMPERED > .shard/objects/\${CID:0:2}/\$CID && shard verify \$CID"
run_scenario "empty_commit_fails" "Empty commit fails" "no" "shard init && shard commit -m 'empty' --author 'T'"
run_scenario "add_nonexistent" "Add bad file fails" "no" "shard init && shard add nonexistent.txt"
run_scenario "checkout_wrong_id" "Bad checkout fails" "no" "shard init && shard checkout 0000000000000000000000000000000000000000000000000000000000000000"
run_scenario "log_no_commits" "Log on empty fails" "no" "shard init && shard log"
run_scenario "tag_bad_commit" "Tag bad commit fails" "no" "shard init && shard tag add badtag 0000000000000000000000000000000000000000000000000000000000000000"

# EDGE CASE
run_scenario "empty_file" "Empty file" "yes" "shard init && cp $MODEL_DIR/empty.dat empty.dat && shard add empty.dat && shard commit -m 'empty' --author 'T'"
run_scenario "one_byte_file" "1-byte file" "yes" "shard init && cp $MODEL_DIR/one_byte.dat onebyte.dat && shard add onebyte.dat && shard commit -m 'onebyte' --author 'T'"
run_scenario "exact_chunk" "4 MiB exactly" "yes" "shard init && cp $MODEL_DIR/exact_chunk.dat . && shard add exact_chunk.dat && shard commit -m 'exact' --author 'T'"
run_scenario "cross_chunk" "4 MiB + 1 byte" "yes" "shard init && cp $MODEL_DIR/cross_chunk.dat . && shard add cross_chunk.dat && shard commit -m 'cross' --author 'T'"
run_scenario "zero_filled" "All zeros" "yes" "shard init && cp $MODEL_DIR/zero_file.dat . && shard add zero_file.dat && shard commit -m 'zeros' --author 'T'"

# ──────────────────────────────────────────────
# Phase 3: Performance Benchmarks
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 3: Performance benchmarks ==="

PERF_DIR="$SCENARIO_DIR/perf"
cleanup_repo "$PERF_DIR"
cd "$PERF_DIR"
shard init >/dev/null

echo "  Benchmarking add throughput..."
for model in tiny small medium large xlarge xxlarge huge; do
    [ -f "$MODEL_DIR/${model}.dat" ] || continue
    size=$(stat --format=%s "$MODEL_DIR/${model}.dat")
    cp "$MODEL_DIR/${model}.dat" "$PERF_DIR/${model}.dat"

    t=$(bench shard add "${model}.dat")
    ms=$(secs_to_ms "$t")
    mib_s=$(awk "BEGIN { printf \"%.2f\", $size / $t / (1024 * 1024) }")
    echo "    add ${model}: ${ms}ms (${mib_s} MiB/s)"
    collect_metric "perf.add_${model}.latency_ms" "$ms" "ms"
    collect_metric "perf.add_${model}.throughput_mibs" "$mib_s" "MiB/s"
done

echo "  Benchmarking commit..."
t=$(bench shard commit -m "bench-all" --author "Bench")
ms=$(secs_to_ms "$t")
echo "    commit: ${ms}ms"
collect_metric "perf.commit.latency_ms" "$ms" "ms"

CID=$(shard log | grep "^commit " | head -1 | awk '{print $2}')

echo "  Benchmarking verify..."
t=$(bench shard verify "$CID")
ms=$(secs_to_ms "$t")
echo "    verify: ${ms}ms"
collect_metric "perf.verify.latency_ms" "$ms" "ms"

echo "  Benchmarking checkout..."
rm -f "$PERF_DIR"/*.dat
t=$(bench shard checkout "$CID")
ms=$(secs_to_ms "$t")
echo "    checkout: ${ms}ms"
collect_metric "perf.checkout.latency_ms" "$ms" "ms"

echo "  Measuring storage efficiency..."
ORIG_SIZE=$(python3 -c "import os; print(sum(os.path.getsize(os.path.join(dp,f)) for dp,dn,fn in os.walk('$MODEL_DIR') for f in fn))" 2>/dev/null || echo 0)
OBJ_SIZE=$(python3 -c "import os; print(sum(os.path.getsize(os.path.join(dp,f)) for dp,dn,fn in os.walk('$PERF_DIR/.shard/objects') for f in fn))" 2>/dev/null || echo 0)
[ -z "$OBJ_SIZE" ] && OBJ_SIZE=0
OBJ_COUNT=$(find "$PERF_DIR/.shard/objects" -type f | wc -l | tr -d ' ')
echo "    Original: $(awk "BEGIN { printf \"%.2f\", $ORIG_SIZE / (1024 * 1024) }") MiB -> Objects: $(awk "BEGIN { printf \"%.2f\", $OBJ_SIZE / (1024 * 1024) }") MiB"
echo "    Object count: $OBJ_COUNT"
if [ "$ORIG_SIZE" -gt 0 ]; then
    overhead=$(awk "BEGIN { printf \"%.2f\", ($OBJ_SIZE - $ORIG_SIZE) / $ORIG_SIZE * 100 }")
    echo "    Overhead: ${overhead}%"
    collect_metric "storage.overhead_pct" "$overhead" "%"
fi
collect_metric "storage.original_bytes" "$ORIG_SIZE" "bytes"
collect_metric "storage.objects_bytes" "$OBJ_SIZE" "bytes"
collect_metric "storage.object_count" "$OBJ_COUNT" "count"

# ──────────────────────────────────────────────
# Phase 4: Large File Throughput Stress
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 4: Large file throughput stress ==="

STRESS_DIR="$SCENARIO_DIR/stress"
cleanup_repo "$STRESS_DIR"
cd "$STRESS_DIR"
shard init >/dev/null

cp "$MODEL_DIR/stress_100m.dat" .
echo "  Adding 100 MiB file..."
t=$(bench shard add stress_100m.dat)
ms=$(secs_to_ms "$t")
mib_s=$(awk "BEGIN { printf \"%.2f\", 100 / $t }")
echo "    add 100 MiB: ${ms}ms (${mib_s} MiB/s)"
collect_metric "stress.add_100m.latency_ms" "$ms" "ms"
collect_metric "stress.add_100m.throughput_mibs" "$mib_s" "MiB/s"

t=$(bench shard commit -m "stress-100m" --author "Stress")
ms=$(secs_to_ms "$t")
echo "    commit 100 MiB: ${ms}ms"
collect_metric "stress.commit_100m.latency_ms" "$ms" "ms"

CID=$(shard log | grep "^commit " | head -1 | awk '{print $2}')
t=$(bench shard verify "$CID")
ms=$(secs_to_ms "$t")
echo "    verify 100 MiB: ${ms}ms"
collect_metric "stress.verify_100m.latency_ms" "$ms" "ms"

STRESS_OBJ_COUNT=$(find "$STRESS_DIR/.shard/objects" -type f | wc -l | tr -d ' ')
echo "    Objects: $STRESS_OBJ_COUNT"
collect_metric "stress.object_count_100m" "$STRESS_OBJ_COUNT" "count"

# ──────────────────────────────────────────────
# Phase 5: Many Small Files Stress
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 5: Many small files stress ==="

MANY_DIR="$SCENARIO_DIR/many"
cleanup_repo "$MANY_DIR"
cd "$MANY_DIR"
shard init >/dev/null

cp -r "$MODEL_DIR/many_small/" .

echo "  Adding 100 files individually..."
t_start=$(date +%s%N)
for f in many_small/*.dat; do
    shard add "$f" >/dev/null 2>&1
done
t_end=$(date +%s%N)
t=$(awk "BEGIN { printf \"%.6f\", ($t_end - $t_start) / 1000000000 }")
ms=$(secs_to_ms "$t")
avg=$(awk "BEGIN { printf \"%.3f\", $ms / 100 }")
echo "    add 100 files: ${ms}ms total, ${avg}ms avg"
collect_metric "many.add_100.latency_ms" "$ms" "ms"
collect_metric "many.add_100.avg_ms" "$avg" "ms"

t_start=$(date +%s%N)
shard commit -m "many-small" --author "Stress" >/dev/null
t_end=$(date +%s%N)
t=$(awk "BEGIN { printf \"%.6f\", ($t_end - $t_start) / 1000000000 }")
ms=$(secs_to_ms "$t")
echo "    commit 100 files: ${ms}ms"
collect_metric "many.commit.latency_ms" "$ms" "ms"

CID=$(shard log | grep "^commit " | head -1 | awk '{print $2}')
t_start=$(date +%s%N)
shard verify "$CID" >/dev/null
t_end=$(date +%s%N)
t=$(awk "BEGIN { printf \"%.6f\", ($t_end - $t_start) / 1000000000 }")
ms=$(secs_to_ms "$t")
echo "    verify 100 files: ${ms}ms"
collect_metric "many.verify.latency_ms" "$ms" "ms"

MANY_OBJ_COUNT=$(find "$MANY_DIR/.shard/objects" -type f | wc -l | tr -d ' ')
echo "    Objects: $MANY_OBJ_COUNT"
collect_metric "many.object_count" "$MANY_OBJ_COUNT" "count"

# ──────────────────────────────────────────────
# Phase 6: P99.99 Latency Distribution
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 6: P99.99 latency distribution ==="

LAT_DIR="$SCENARIO_DIR/latency"
cleanup_repo "$LAT_DIR"
cd "$LAT_DIR"
shard init >/dev/null

TEMPFILE="$LAT_DIR/_bench.dat"

# pre-generate unique files for each iteration to avoid hash collisions
for i in $(seq 1 50); do
    dd if=/dev/urandom of="$LAT_DIR/_file_${i}.dat" bs=1K count=64 2>/dev/null
done

N=50
echo "  Running $N add-commit-verify cycles..."
rm -f /tmp/_lat_*.txt

for i in $(seq 1 $N); do
    cp "$LAT_DIR/_file_${i}.dat" "$LAT_DIR/bench.dat"

    t_add=$(bench shard add bench.dat)
    ms_add=$(secs_to_ms "$t_add")
    echo "$ms_add" >> /tmp/_lat_add.txt

    t_commit=$(bench shard commit -m "lat-${i}" --author "Lat")
    ms_commit=$(secs_to_ms "$t_commit")
    echo "$ms_commit" >> /tmp/_lat_commit.txt

    CID=$(shard log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
    t_verify=$(bench shard verify "$CID")
    ms_verify=$(secs_to_ms "$t_verify")
    echo "$ms_verify" >> /tmp/_lat_verify.txt

    rm -f "$LAT_DIR/bench.dat"
    echo "    [$i/$N] add=${ms_add}ms commit=${ms_commit}ms verify=${ms_verify}ms"
done

percentile() {
    local file="$1"
    local p="$2"
    sort -n "$file" | awk -v p="$p" 'BEGIN{c=0} {vals[c++]=$1} END{idx=int(c*p/100); if(idx>=c) idx=c-1; print vals[idx]}'
}

echo ""
echo "  Latency Distribution (ms):"
for op in add commit verify; do
    file="/tmp/_lat_${op}.txt"
    [ -f "$file" ] || continue
    sort -n "$file" -o "$file"
    p50=$(percentile "$file" 50)
    p90=$(percentile "$file" 90)
    p99=$(percentile "$file" 99)
    p999=$(percentile "$file" 99.9)
    p9999=$(percentile "$file" 99.99)
    mean=$(awk '{s+=$1} END{printf "%.2f", s/NR}' "$file")
    min=$(head -1 "$file")
    max=$(tail -1 "$file")
    echo "    ${op}: min=${min}ms p50=${p50}ms p90=${p90}ms p99=${p99}ms p99.9=${p999}ms p99.99=${p9999}ms max=${max}ms mean=${mean}ms"
    for q in min p50 p90 p99 p999 p9999 max mean; do
        case $q in
            min) v=$min ;; p50) v=$p50 ;; p90) v=$p90 ;; p99) v=$p99 ;;
            p999) v=$p999 ;; p9999) v=$p9999 ;; max) v=$max ;; mean) v=$mean ;;
        esac
        collect_metric "latency.${op}.${q}" "$v" "ms"
    done
done

# ──────────────────────────────────────────────
# Phase 7: Storage Scaling
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 7: Storage scaling ==="

SCALE_DIR="$SCENARIO_DIR/scale"
cleanup_repo "$SCALE_DIR"
cd "$SCALE_DIR"
shard init >/dev/null

echo "  Measuring storage per file size..."
SCALE_CSV="$RESULTS_DIR/scale.csv"
echo "file_size_mib,object_count,object_bytes,original_bytes,overhead_pct" > "$SCALE_CSV"

for size_mib in 1 4 8 16 32; do
    sf="scale_${size_mib}m.dat"
    dd if=/dev/urandom of="$sf" bs=1M count="$size_mib" 2>/dev/null
    orig_bytes=$(stat --format=%s "$sf")
    shard add "$sf" >/dev/null 2>&1
    shard commit -m "scale-${size_mib}m" --author "Scale" >/dev/null 2>&1

    obj_bytes=$(python3 -c "import os; print(sum(os.path.getsize(os.path.join(dp,f)) for dp,dn,fn in os.walk('.shard/objects') for f in fn))" 2>/dev/null || echo 0)
    obj_count=$(find .shard/objects -type f | wc -l | tr -d ' ')
    overhead=$(awk "BEGIN { printf \"%.2f\", ($obj_bytes - $orig_bytes) / $orig_bytes * 100 }")
    echo "$size_mib,$obj_count,$obj_bytes,$orig_bytes,$overhead" >> "$SCALE_CSV"
    echo "    ${size_mib} MiB: ${obj_count} objects, ${overhead}% overhead"
done

collect_metric "storage.scaling_count" "$(wc -l < "$SCALE_CSV")" "count"

# ──────────────────────────────────────────────
# Phase 8: Network P2P Throughput
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 8: Network P2P throughput ==="

P2P_DIR="$SCENARIO_DIR/p2p"
cleanup_repo "${P2P_DIR}/server"
cd "${P2P_DIR}/server"
shard init >/dev/null

dd if=/dev/urandom of="share_file.dat" bs=1M count=4 2>/dev/null
shard add share_file.dat >/dev/null
shard commit -m "share-test" --author "Net" >/dev/null
CID_NET=$(shard log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')

SHARE_LOG="${P2P_DIR}/share.log"
"$SHARD_BIN" share > "$SHARE_LOG" 2>&1 &
SHARE_PID=$!

LISTEN_ADDR=""
PEER_ID=""
for i in $(seq 1 30); do
    LISTEN_ADDR=$(grep "Listening on" "$SHARE_LOG" 2>/dev/null | head -1 | awk '{print $3}' | tr -d '"' || true)
    PEER_ID=$(grep "Local peer id:" "$SHARE_LOG" 2>/dev/null | head -1 | awk '{print $4}' || true)
    [ -n "$LISTEN_ADDR" ] && [ -n "$PEER_ID" ] && break
    sleep 1
done
MULTIADDR="$LISTEN_ADDR/p2p/$PEER_ID"
echo "  Server: $MULTIADDR"

cleanup_repo "${P2P_DIR}/client"
cd "${P2P_DIR}/client"
shard init >/dev/null

echo "  Measuring pull throughput..."
t=$(bench "$SHARD_BIN" pull "$MULTIADDR" "$CID_NET")
ms=$(secs_to_ms "$t")
mib_s=$(awk "BEGIN { printf \"%.2f\", 4 / $t }")
echo "    pull 4 MiB: ${ms}ms (${mib_s} MiB/s)"
collect_metric "net.pull_4m.latency_ms" "$ms" "ms"
collect_metric "net.pull_4m.throughput_mibs" "$mib_s" "MiB/s"

kill "$SHARE_PID" 2>/dev/null || true
wait "$SHARE_PID" 2>/dev/null || true

# ──────────────────────────────────────────────
# Phase 9: Concurrent Write Stress
# ──────────────────────────────────────────────
echo ""
echo "=== Phase 9: Concurrent init/add stress ==="

CONC_DIR="$SCENARIO_DIR/concurrent"
cleanup_repo "$CONC_DIR"
cd "$CONC_DIR"
echo "  Testing concurrent repository operations..."

# Run 5 independent repos simultaneously
for i in $(seq 1 5); do
    rd="$CONC_DIR/repo_$i"
    mkdir -p "$rd"
    (
        cd "$rd"
        shard init >/dev/null 2>&1
        dd if=/dev/urandom of="test.dat" bs=1M count=1 2>/dev/null
        shard add test.dat >/dev/null 2>&1
        shard commit -m "conc-$i" --author "Conc" >/dev/null 2>&1
    ) &
done
wait

OK_COUNT=0
for i in $(seq 1 5); do
    if [ -f "repo_$i/.shard/HEAD" ]; then
        OK_COUNT=$((OK_COUNT + 1))
    fi
done
echo "    Concurrent inits completed: ${OK_COUNT}/5"
collect_metric "concurrent.init_success" "$OK_COUNT" "count"

# ──────────────────────────────────────────────
# Finalize metrics JSON
# ──────────────────────────────────────────────
echo "" >> "$RESULTS_DIR/metrics.json"
echo "}" >> "$RESULTS_DIR/metrics.json"

# Fix JSON by reading all entries and rewriting properly
python3 -c "
import json, re
with open('$RESULTS_DIR/metrics.json') as f:
    content = f.read()
# Extract all key-value pairs using regex
# Pattern: \"key\": {...}
pairs = re.findall(r'\"([^\"]+)\":\s*(\{[^\}]+\})', content)
entries = {}
for k, v in pairs:
    try:
        entries[k] = json.loads(v)
    except json.JSONDecodeError:
        pass
# Handle model_files specially if present
mf_match = re.search(r'\"model_files\":\s*(\{[^\}]+\})', content)
if mf_match:
    try:
        entries['model_files'] = json.loads(mf_match.group(1))
    except json.JSONDecodeError:
        pass
with open('$RESULTS_DIR/metrics.json', 'w') as f:
    json.dump(entries, f, indent=2)
print(f'  metrics.json valid: {len(entries)} entries')
" 2>&1

echo ""
echo "═══════════════════════════════════════════"
echo "  ALL STRESS TESTS COMPLETE"
echo "  Results: $RESULTS_DIR"
echo "═══════════════════════════════════════════"

echo "$RESULTS_DIR"
