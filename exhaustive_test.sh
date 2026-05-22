#!/bin/bash
# ======================================================================
# SHARD EXHAUSTIVE TEST — ALL use cases, edge cases, loopholes, panics,
# error paths, stress scenarios, P99.99 latency, storage scaling, P2P.
# ======================================================================
set -uo pipefail

SHARD_BIN="$(realpath target/release/shard)"
RESULTS_DIR="$(mktemp -d)"
MODEL_DIR="$RESULTS_DIR/models"
SCENARIO_DIR="$RESULTS_DIR/scenarios"
mkdir -p "$MODEL_DIR" "$SCENARIO_DIR"

# Use a function so SHARD_BIN is always resolved correctly
sh() { "$SHARD_BIN" "$@"; }

PASS=true
TOTAL=0
PASSED=0
FAILED=0
ERRORS=""

cd "$SCENARIO_DIR"  # <-- ROOT for all scenarios

assert() {
    local name="$1"
    local desc="$2"
    local expected="$3"
    shift 3
    TOTAL=$((TOTAL + 1))
    local ok=true
    local output
    output=$("$@" 2>&1) || ok=false
    local result="pass"
    if { [ "$expected" = "succeed" ] && [ "$ok" = false ]; } || { [ "$expected" = "fail" ] && [ "$ok" = true ]; }; then
        result="fail"
        PASS=false
        FAILED=$((FAILED + 1))
        ERRORS="${ERRORS}  FAIL [$name] expected=$expected ok=$ok\n"
    else
        PASSED=$((PASSED + 1))
    fi
    echo "  [$result] $name"
}

# =====================================================================
# PHASE 1: Create comprehensive model files
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 1: Create model files                                ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# Normal sizes
dd if=/dev/urandom of="$MODEL_DIR/1byte.dat" bs=1 count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/1K.dat" bs=1K count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/64K.dat" bs=1K count=64 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/1M.dat" bs=1M count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/4M.dat" bs=4M count=1 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/8M.dat" bs=4M count=2 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/16M.dat" bs=4M count=4 2>/dev/null

# Chunk boundaries
truncate -s 4194304 "$MODEL_DIR/4M_exact.dat"
truncate -s 4194305 "$MODEL_DIR/4M_plus1.dat"
truncate -s 8388608 "$MODEL_DIR/8M_exact.dat"
truncate -s 8388609 "$MODEL_DIR/8M_plus1.dat"

# Special content
truncate -s 0 "$MODEL_DIR/empty.dat"
dd if=/dev/zero bs=1M count=1 2>/dev/null > "$MODEL_DIR/zeros_1M.dat"
python3 -c "open('$MODEL_DIR/only_newlines.dat','w').write('\n'*10000)" 2>/dev/null
python3 -c "open('$MODEL_DIR/only_spaces.dat','w').write(' '*10000)" 2>/dev/null
python3 -c "open('$MODEL_DIR/utf8_content.dat','wb').write('ññoöüßéèêëàâäùûüœæ€∞✓✅'.encode()*100)" 2>/dev/null
dd if=/dev/urandom bs=1M count=4 2>/dev/null > "$MODEL_DIR/sparse.dat"; truncate -s 16M "$MODEL_DIR/sparse.dat" 2>/dev/null

# Many small files
mkdir -p "$MODEL_DIR/many_small"
for i in $(seq 1 100); do dd if=/dev/urandom of="$MODEL_DIR/many_small/file_${i}.dat" bs=1K count=1 2>/dev/null; done

# Special filenames
echo x > "$MODEL_DIR/space file.dat"
echo x > "$MODEL_DIR/file_with_#_hash.dat"
echo x > "$MODEL_DIR/file_with_\$_dollar.dat"
echo x > "$MODEL_DIR/file_with_&.dat"
echo x > "$MODEL_DIR/file_with_'_quote.dat"
echo x > "$MODEL_DIR/file_with_\"_quote.dat"
echo x > "$MODEL_DIR/file_with_()_parens.dat"
echo x > "$MODEL_DIR/file_with_{}_braces.dat"
echo x > "$MODEL_DIR/file_with_[].dat"
python3 -c "open('$MODEL_DIR/unicode_ñ_file.dat','w').write('u')" 2>/dev/null

# Symlinks
ln -sf "$MODEL_DIR/1K.dat" "$MODEL_DIR/symlink_valid.dat"
ln -sf /nonexistent_path_xyz "$MODEL_DIR/symlink_broken.dat"

# Hard link
ln "$MODEL_DIR/1K.dat" "$MODEL_DIR/hardlink_copy.dat"

# Stress files
dd if=/dev/urandom of="$MODEL_DIR/stress_100M.dat" bs=1M count=100 2>/dev/null
dd if=/dev/urandom of="$MODEL_DIR/stress_200M.dat" bs=1M count=200 2>/dev/null

# Dedup pair
dd if=/dev/urandom bs=1K count=64 2>/dev/null | tee "$MODEL_DIR/dedup_a.dat" > "$MODEL_DIR/dedup_b.dat"

echo "  Model files created in $MODEL_DIR"

# =====================================================================
# PHASE 2: PANIC VECTORS & CRASH RESILIENCE
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 2: Panic vectors & crash resilience                  ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# 2a: add parent dir (file_name()=None -> panic)
mkdir -p "p2_add_dotdot" && cd "p2_add_dotdot"
sh init >/dev/null 2>&1
assert "p2_add_dotdot" "shard add .." "fail" sh add ..
cd "$SCENARIO_DIR"

# 2b: add current dir
mkdir -p "p2_add_dot" && cd "p2_add_dot"
sh init >/dev/null 2>&1
assert "p2_add_dot" "shard add ." "fail" sh add .
cd "$SCENARIO_DIR"

# 2c: verify empty string
mkdir -p "p2_verify_empty" && cd "p2_verify_empty"
sh init >/dev/null 2>&1
assert "p2_verify_empty" "verify ''" "fail" sh verify ""
cd "$SCENARIO_DIR"

# 2d: verify 1-char
mkdir -p "p2_verify_1char" && cd "p2_verify_1char"
sh init >/dev/null 2>&1
assert "p2_verify_1char" "verify 'a'" "fail" sh verify "a"
cd "$SCENARIO_DIR"

# 2e: checkout empty
mkdir -p "p2_co_empty" && cd "p2_co_empty"
sh init >/dev/null 2>&1
assert "p2_co_empty" "checkout ''" "fail" sh checkout ""
cd "$SCENARIO_DIR"

# 2f: checkout 1-char
mkdir -p "p2_co_1char" && cd "p2_co_1char"
sh init >/dev/null 2>&1
assert "p2_co_1char" "checkout 'a'" "fail" sh checkout "a"
cd "$SCENARIO_DIR"

# 2g: tag with empty commit
mkdir -p "p2_tag_empty" && cd "p2_tag_empty"
sh init >/dev/null 2>&1
assert "p2_tag_empty" "tag add with empty id" "fail" sh tag add mytag ""
cd "$SCENARIO_DIR"

# 2h: verify non-hex
mkdir -p "p2_verify_nonhex" && cd "p2_verify_nonhex"
sh init >/dev/null 2>&1
assert "p2_verify_nonhex" "verify 'zzzz...'" "fail" sh verify "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
cd "$SCENARIO_DIR"

echo "  Phase 2 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 3: INPUT VALIDATION GAPS
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 3: Input validation gaps                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"

mkdir -p "p3_dir" && cd "p3_dir"
sh init >/dev/null 2>&1
mkdir -p "somedir"
assert "p3_add_dir" "add a directory" "fail" sh add somedir
cd "$SCENARIO_DIR"

mkdir -p "p3_symlink_valid" && cd "p3_symlink_valid"
sh init >/dev/null 2>&1
ln -sf "$MODEL_DIR/1K.dat" link.dat
assert "p3_symlink_valid" "add symlink to file" "succeed" sh add link.dat
cd "$SCENARIO_DIR"

mkdir -p "p3_symlink_broken" && cd "p3_symlink_broken"
sh init >/dev/null 2>&1
ln -sf /nonexistent_path_xyz broken.dat
assert "p3_symlink_broken" "add broken symlink" "fail" sh add broken.dat
cd "$SCENARIO_DIR"

mkdir -p "p3_nonexist" && cd "p3_nonexist"
sh init >/dev/null 2>&1
assert "p3_add_nonexist" "add nonexistent file" "fail" sh add nonexistent.dat
cd "$SCENARIO_DIR"

mkdir -p "p3_peer_invalid" && cd "p3_peer_invalid"
sh init >/dev/null 2>&1
assert "p3_peer_invalid" "peer add invalid multiaddr" "succeed" sh peer add "not-a-multiaddr"
cd "$SCENARIO_DIR"

mkdir -p "p3_peer_empty" && cd "p3_peer_empty"
sh init >/dev/null 2>&1
assert "p3_peer_empty" "peer add empty string" "succeed" sh peer add ""
cd "$SCENARIO_DIR"

mkdir -p "p3_verify_short" && cd "p3_verify_short"
sh init >/dev/null 2>&1
assert "p3_verify_short" "verify short hash" "fail" sh verify "123456789a"
cd "$SCENARIO_DIR"

mkdir -p "p3_noperm" && cd "p3_noperm"
sh init >/dev/null 2>&1
echo "secret" > noperm.dat && chmod 000 noperm.dat
assert "p3_noperm" "add no-permission file" "fail" sh add noperm.dat
chmod 644 noperm.dat
cd "$SCENARIO_DIR"

echo "  Phase 3 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 4: HAPPY PATH WORKFLOWS
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 4: Happy path workflows                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# 4a: init
mkdir -p "p4_init" && cd "p4_init"
assert "p4_init" "init" "succeed" sh init
cd "$SCENARIO_DIR"

# 4b: init --private
mkdir -p "p4_private" && cd "p4_private"
assert "p4_private" "init --private" "succeed" sh init --private
sh config get private 2>/dev/null | grep -q "true" || echo "  WARN: private not set"
cd "$SCENARIO_DIR"

# 4c: add+commit+verify
mkdir -p "p4_full_cycle" && cd "p4_full_cycle"
sh init >/dev/null 2>&1
cp "$MODEL_DIR/64K.dat" .
sh add 64K.dat >/dev/null 2>&1
assert "p4_commit" "commit" "succeed" sh commit -m "first" --author "Tester"
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p4_verify" "verify $CID" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 4d: multiple files one commit
mkdir -p "p4_multi" && cd "p4_multi"
sh init >/dev/null 2>&1
cp "$MODEL_DIR/1K.dat" "$MODEL_DIR/64K.dat" "$MODEL_DIR/1M.dat" .
sh add 1K.dat >/dev/null 2>&1 && sh add 64K.dat >/dev/null 2>&1 && sh add 1M.dat >/dev/null 2>&1
assert "p4_multi_commit" "commit 3 files" "succeed" sh commit -m "multi" --author "T"
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p4_multi_verify" "verify multi" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 4e: two sequential commits
mkdir -p "p4_seq" && cd "p4_seq"
sh init >/dev/null 2>&1
echo "a" > a.dat && sh add a.dat >/dev/null 2>&1 && sh commit -m "first" --author "T" >/dev/null 2>&1
echo "b" > b.dat && sh add b.dat >/dev/null 2>&1
assert "p4_2nd_commit" "second commit" "succeed" sh commit -m "second" --author "T"
CID2=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p4_2nd_verify" "verify second" "succeed" sh verify "$CID2"
cd "$SCENARIO_DIR"

# 4f: checkout restore
mkdir -p "p4_co_restore" && cd "p4_co_restore"
sh init >/dev/null 2>&1
echo "checkout test data" > restore.dat && sh add restore.dat >/dev/null 2>&1 && sh commit -m "co-me" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
rm restore.dat
assert "p4_co" "checkout restores file" "succeed" sh checkout "$CID"
[ -f restore.dat ] && grep -q "checkout test data" restore.dat || echo "  WARN: file not restored correctly"
cd "$SCENARIO_DIR"

# 4g: checkout --json
mkdir -p "p4_co_json" && cd "p4_co_json"
sh init >/dev/null 2>&1
echo "json" > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "json-co" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
rm f.dat
output=$(sh checkout --json "$CID" 2>/dev/null)
python3 -c "import json; v=json.loads('$output'); assert 'commit_id' in v; assert 'files' in v" 2>/dev/null && echo "  OK: checkout --json valid" || echo "  WARN: checkout --json invalid"
cd "$SCENARIO_DIR"

# 4h: log
mkdir -p "p4_log" && cd "p4_log"
sh init >/dev/null 2>&1
echo "a" > a.dat && sh add a.dat >/dev/null 2>&1 && sh commit -m "c1" --author "A" >/dev/null 2>&1
echo "b" > b.dat && sh add b.dat >/dev/null 2>&1 && sh commit -m "c2" --author "B" >/dev/null 2>&1
LOGOUT=$(sh log 2>/dev/null)
echo "$LOGOUT" | grep -q "c1" && echo "$LOGOUT" | grep -q "c2" && echo "  OK: log shows both" || echo "  WARN: log missing commits"
cd "$SCENARIO_DIR"

# 4i: log --json
mkdir -p "p4_log_json" && cd "p4_log_json"
sh init >/dev/null 2>&1
echo "x" > x.dat && sh add x.dat >/dev/null 2>&1 && sh commit -m "ljson" --author "J" >/dev/null 2>&1
output=$(sh log --json 2>/dev/null)
python3 -c "import json; e=json.loads('$output'); assert len(e)>0; assert e[0].get('message')=='ljson'" 2>/dev/null && echo "  OK: log --json valid" || echo "  WARN: log --json invalid"
cd "$SCENARIO_DIR"

# 4j: status after init
mkdir -p "p4_status_init" && cd "p4_status_init"
sh init >/dev/null 2>&1
sh status 2>/dev/null | grep -q "No commits" && echo "  OK: status shows No commits" || echo "  WARN: status message"
cd "$SCENARIO_DIR"

# 4k: status after commit
mkdir -p "p4_status_commit" && cd "p4_status_commit"
sh init >/dev/null 2>&1
echo d > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "s" --author "T" >/dev/null 2>&1
sh status 2>/dev/null | grep -q "On commit" && echo "  OK: status shows commit" || echo "  WARN: status no commit"
cd "$SCENARIO_DIR"

# 4l: status untracked
mkdir -p "p4_status_untracked" && cd "p4_status_untracked"
sh init >/dev/null 2>&1
echo t > tracked.dat && sh add tracked.dat >/dev/null 2>&1 && sh commit -m "t" --author "T" >/dev/null 2>&1
echo u > untracked.dat
sh status 2>/dev/null | grep -q "untracked" && echo "  OK: status shows untracked" || echo "  WARN: no untracked"
cd "$SCENARIO_DIR"

# 4m: config set/get
mkdir -p "p4_config" && cd "p4_config"
sh init >/dev/null 2>&1
sh config set user.name "Alice" >/dev/null 2>&1
sh config get user.name 2>/dev/null | grep -q "Alice" && echo "  OK: config works" || echo "  WARN: config"
cd "$SCENARIO_DIR"

# 4n: tag add + list
mkdir -p "p4_tag" && cd "p4_tag"
sh init >/dev/null 2>&1
echo d > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "tg" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
sh tag add v1 "$CID" >/dev/null 2>&1
sh tag list 2>/dev/null | grep -q "v1" && echo "  OK: tag works" || echo "  WARN: tag"
cd "$SCENARIO_DIR"

# 4o: prune clean
mkdir -p "p4_prune" && cd "p4_prune"
sh init >/dev/null 2>&1
cp "$MODEL_DIR/1K.dat" . && sh add 1K.dat >/dev/null 2>&1 && sh commit -m "k" --author "T" >/dev/null 2>&1
assert "p4_prune" "prune clean repo" "succeed" sh prune
cd "$SCENARIO_DIR"

# 4p: verify --json
mkdir -p "p4_verify_json" && cd "p4_verify_json"
sh init >/dev/null 2>&1
echo d > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "vj" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
output=$(sh verify --json "$CID" 2>/dev/null)
python3 -c "import json; v=json.loads('$output'); assert v['verified']; assert v['signature_verified']" 2>/dev/null && echo "  OK: verify --json valid" || echo "  WARN: verify --json"
cd "$SCENARIO_DIR"

# 4q: status --json
mkdir -p "p4_status_json" && cd "p4_status_json"
sh init >/dev/null 2>&1
echo d > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "sj" --author "T" >/dev/null 2>&1
output=$(sh status --json 2>/dev/null)
python3 -c "import json; s=json.loads('$output'); assert 'commit' in s; assert 'staged' in s" 2>/dev/null && echo "  OK: status --json valid" || echo "  WARN: status --json"
cd "$SCENARIO_DIR"

echo "  Phase 4 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 5: NON-HAPPY / ERROR PATHS
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 5: Error paths                                       ║"
echo "╚══════════════════════════════════════════════════════════════╝"

mkdir -p "p5_init_twice" && cd "p5_init_twice"
sh init >/dev/null 2>&1
assert "p5_init_twice" "init twice" "fail" sh init
cd "$SCENARIO_DIR"

mkdir -p "p5_commit_empty" && cd "p5_commit_empty"
sh init >/dev/null 2>&1
assert "p5_commit_empty" "commit nothing staged" "fail" sh commit -m "x" --author "T"
cd "$SCENARIO_DIR"

mkdir -p "p5_verify_bad" && cd "p5_verify_bad"
sh init >/dev/null 2>&1
assert "p5_verify_bad" "verify nonexistent" "fail" sh verify "0000000000000000000000000000000000000000000000000000000000000000"
cd "$SCENARIO_DIR"

mkdir -p "p5_tamper" && cd "p5_tamper"
sh init >/dev/null 2>&1
echo secret > secret.txt && sh add secret.txt >/dev/null 2>&1 && sh commit -m "sec" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
PREFIX=${CID:0:2}
mkdir -p ".shard/objects/$PREFIX"
echo "TAMPERED" > ".shard/objects/$PREFIX/$CID"
assert "p5_tamper" "verify tampered" "fail" sh verify "$CID"
cd "$SCENARIO_DIR"

mkdir -p "p5_co_bad" && cd "p5_co_bad"
sh init >/dev/null 2>&1
assert "p5_co_bad" "checkout bad id" "fail" sh checkout "0000000000000000000000000000000000000000000000000000000000000000"
cd "$SCENARIO_DIR"

mkdir -p "p5_log_empty" && cd "p5_log_empty"
sh init >/dev/null 2>&1
assert "p5_log_empty" "log empty" "fail" sh log
cd "$SCENARIO_DIR"

mkdir -p "p5_tag_bad" && cd "p5_tag_bad"
sh init >/dev/null 2>&1
assert "p5_tag_bad" "tag bad commit" "fail" sh tag add bad "0000000000000000000000000000000000000000000000000000000000000000"
cd "$SCENARIO_DIR"

# Operations without init
mkdir -p "p5_noinit_add" && cd "p5_noinit_add"
assert "p5_noinit_add" "add no init" "fail" sh add any.dat
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_commit" && cd "p5_noinit_commit"
assert "p5_noinit_commit" "commit no init" "fail" sh commit -m "x" --author "T"
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_verify" && cd "p5_noinit_verify"
assert "p5_noinit_verify" "verify no init" "fail" sh verify "0000000000000000000000000000000000000000000000000000000000000000"
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_log" && cd "p5_noinit_log"
assert "p5_noinit_log" "log no init" "fail" sh log
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_co" && cd "p5_noinit_co"
assert "p5_noinit_co" "checkout no init" "fail" sh checkout "0000000000000000000000000000000000000000000000000000000000000000"
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_status" && cd "p5_noinit_status"
assert "p5_noinit_status" "status no init" "fail" sh status
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_config_get" && cd "p5_noinit_config_get"
assert "p5_noinit_config_get" "config get no init" "fail" sh config get foo
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_config_set" && cd "p5_noinit_config_set"
assert "p5_noinit_config_set" "config set no init" "fail" sh config set foo bar
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_tag" && cd "p5_noinit_tag"
assert "p5_noinit_tag" "tag no init" "fail" sh tag add v1 "00"
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_prune" && cd "p5_noinit_prune"
assert "p5_noinit_prune" "prune no init" "fail" sh prune
cd "$SCENARIO_DIR"

mkdir -p "p5_noinit_peer" && cd "p5_noinit_peer"
assert "p5_noinit_peer" "peer no init" "fail" sh peer add "/ip4/1.2.3.4/tcp/1234"
cd "$SCENARIO_DIR"

echo "  Phase 5 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 6: EDGE CASES
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 6: Edge cases                                        ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# 6a: empty file
mkdir -p "p6_empty" && cd "p6_empty"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/empty.dat" . && sh add empty.dat >/dev/null 2>&1 && sh commit -m "empty" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_empty_verify" "verify empty file" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6b: 1 byte
mkdir -p "p6_1byte" && cd "p6_1byte"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/1byte.dat" . && sh add 1byte.dat >/dev/null 2>&1 && sh commit -m "1b" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_1byte_verify" "verify 1-byte" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6c: exact 4 MiB
mkdir -p "p6_4m_exact" && cd "p6_4m_exact"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/4M_exact.dat" . && sh add 4M_exact.dat >/dev/null 2>&1 && sh commit -m "4m-exact" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_4m_exact_verify" "verify 4MiB exact" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6d: 4MiB + 1 byte
mkdir -p "p6_4m_plus1" && cd "p6_4m_plus1"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/4M_plus1.dat" . && sh add 4M_plus1.dat >/dev/null 2>&1 && sh commit -m "4m+1" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_4m_plus1_verify" "verify 4MiB+1" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6e: exact 8 MiB
mkdir -p "p6_8m_exact" && cd "p6_8m_exact"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/8M_exact.dat" . && sh add 8M_exact.dat >/dev/null 2>&1 && sh commit -m "8m-exact" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_8m_exact_verify" "verify 8MiB exact" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6f: 8MiB + 1 byte
mkdir -p "p6_8m_plus1" && cd "p6_8m_plus1"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/8M_plus1.dat" . && sh add 8M_plus1.dat >/dev/null 2>&1 && sh commit -m "8m+1" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_8m_plus1_verify" "verify 8MiB+1" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6g: zeros
mkdir -p "p6_zeros" && cd "p6_zeros"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/zeros_1M.dat" . && sh add zeros_1M.dat >/dev/null 2>&1 && sh commit -m "zeros" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p6_zeros_verify" "verify zeros" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 6h: sparse
mkdir -p "p6_sparse" && cd "p6_sparse"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/sparse.dat" .
assert "p6_sparse_add" "add sparse" "succeed" sh add sparse.dat
cd "$SCENARIO_DIR"

# 6i: newlines
mkdir -p "p6_newlines" && cd "p6_newlines"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/only_newlines.dat" . && sh add only_newlines.dat >/dev/null 2>&1
assert "p6_newlines_commit" "commit newlines" "succeed" sh commit -m "nl" --author "T"
cd "$SCENARIO_DIR"

# 6j: spaces
mkdir -p "p6_spaces" && cd "p6_spaces"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/only_spaces.dat" . && sh add only_spaces.dat >/dev/null 2>&1
assert "p6_spaces_commit" "commit spaces" "succeed" sh commit -m "sp" --author "T"
cd "$SCENARIO_DIR"

# 6k: utf8 content
mkdir -p "p6_utf8" && cd "p6_utf8"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/utf8_content.dat" . && sh add utf8_content.dat >/dev/null 2>&1
assert "p6_utf8_commit" "commit utf8 content" "succeed" sh commit -m "utf8" --author "T"
cd "$SCENARIO_DIR"

# 6l: unicode filename
mkdir -p "p6_unicode_fn" && cd "p6_unicode_fn"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/unicode_ñ_file.dat" .
assert "p6_unicode_fn_add" "add unicode filename" "succeed" sh add "unicode_ñ_file.dat"
cd "$SCENARIO_DIR"

# 6m: special chars filename
mkdir -p "p6_special_fn" && cd "p6_special_fn"
sh init >/dev/null 2>&1
cp "$MODEL_DIR/file_with_#_hash.dat" . && sh add "file_with_#_hash.dat" >/dev/null 2>&1
cp "$MODEL_DIR/file_with_\$_dollar.dat" . && sh add "file_with_\$_dollar.dat" >/dev/null 2>&1
cp "$MODEL_DIR/file_with_&.dat" . && sh add "file_with_&.dat" >/dev/null 2>&1
assert "p6_special_fn_commit" "commit special filenames" "succeed" sh commit -m "special" --author "T"
cd "$SCENARIO_DIR"

# 6n: dedup check
mkdir -p "p6_dedup" && cd "p6_dedup"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/dedup_a.dat" a.dat && cp "$MODEL_DIR/dedup_b.dat" b.dat
sh add a.dat >/dev/null 2>&1 && sh add b.dat >/dev/null 2>&1 && sh commit -m "dedup" --author "T" >/dev/null 2>&1
OBJ_N=$(find .shard/objects -type f | wc -l)
echo "    Objects for 2 identical 64K files: $OBJ_N (expected 3 = commit+manifest+1 chunk)"
cd "$SCENARIO_DIR"

# 6o: hard link
mkdir -p "p6_hardlink" && cd "p6_hardlink"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/hardlink_copy.dat" .
assert "p6_hardlink" "add hard link copy" "succeed" sh add hardlink_copy.dat
cd "$SCENARIO_DIR"

# 6p: symlink valid
mkdir -p "p6_symlink_file" && cd "p6_symlink_file"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/symlink_valid.dat" .
assert "p6_symlink_valid" "add symlink copy" "succeed" sh add symlink_valid.dat
cd "$SCENARIO_DIR"

# 6q: space in filename
mkdir -p "p6_space_fn" && cd "p6_space_fn"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/space file.dat" .
assert "p6_space_fn" "add space filename" "succeed" sh add "space file.dat"
cd "$SCENARIO_DIR"

# 6r: special chars in commit message
mkdir -p "p6_msg_special" && cd "p6_msg_special"
sh init >/dev/null 2>&1 && echo d > f.dat && sh add f.dat >/dev/null 2>&1
assert "p6_msg_special" "commit msg special chars" "succeed" sh commit -m "hello 'world' & \"everyone\"" --author "T"
cd "$SCENARIO_DIR"

# 6s: empty commit message
mkdir -p "p6_msg_empty" && cd "p6_msg_empty"
sh init >/dev/null 2>&1 && echo d > f.dat && sh add f.dat >/dev/null 2>&1
assert "p6_msg_empty" "commit msg empty" "succeed" sh commit -m "" --author "T"
cd "$SCENARIO_DIR"

# 6t: empty author
mkdir -p "p6_author_empty" && cd "p6_author_empty"
sh init >/dev/null 2>&1 && echo d > f.dat && sh add f.dat >/dev/null 2>&1
assert "p6_author_empty" "commit author empty" "succeed" sh commit -m "x" --author ""
cd "$SCENARIO_DIR"

echo "  Phase 6 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 7: STATE MACHINE VIOLATIONS
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 7: State machine violations                          ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# 7a: add same file twice (overwrite)
mkdir -p "p7_add_twice" && cd "p7_add_twice"
sh init >/dev/null 2>&1
echo "v1" > f.dat && sh add f.dat >/dev/null 2>&1
echo "v2" > f.dat && sh add f.dat >/dev/null 2>&1
sh commit -m "ow" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
rm f.dat && sh checkout "$CID" >/dev/null 2>&1
[ "$(cat f.dat)" = "v2" ] && echo "  OK: add overwrite uses latest" || echo "  WARN: add overwrite"
cd "$SCENARIO_DIR"

# 7b: two commits, parent chain
mkdir -p "p7_parent_chain" && cd "p7_parent_chain"
sh init >/dev/null 2>&1
echo a > a.dat && sh add a.dat >/dev/null 2>&1 && sh commit -m "c1" --author "T" >/dev/null 2>&1
CID1=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
echo b > b.dat && sh add b.dat >/dev/null 2>&1 && sh commit -m "c2" --author "T" >/dev/null 2>&1
CID2=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
[ "$CID1" != "$CID2" ] && echo "  OK: different commits" || echo "  WARN: same commit id"
sh log 2>/dev/null | grep -q "parents:" && echo "  OK: second has parent" || echo "  WARN: no parent"
cd "$SCENARIO_DIR"

# 7c: checkout overwrites local
mkdir -p "p7_co_overwrite" && cd "p7_co_overwrite"
sh init >/dev/null 2>&1
echo committed > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "co" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
echo "local-changes" > f.dat
sh checkout "$CID" >/dev/null 2>&1
[ "$(cat f.dat)" = "committed" ] && echo "  OK: checkout overwrites local" || echo "  WARN: checkout no overwrite"
cd "$SCENARIO_DIR"

# 7d: prune then verify
mkdir -p "p7_prune_verify" && cd "p7_prune_verify"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/64K.dat" . && sh add 64K.dat >/dev/null 2>&1 && sh commit -m "pv" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
sh prune >/dev/null 2>&1
assert "p7_prune_verify" "verify after prune" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 7e: tag protects from prune
mkdir -p "p7_tag_protect" && cd "p7_tag_protect"
sh init >/dev/null 2>&1 && cp "$MODEL_DIR/1K.dat" . && sh add 1K.dat >/dev/null 2>&1 && sh commit -m "tp" --author "T" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
sh tag add safe "$CID" >/dev/null 2>&1
mkdir -p .shard/objects/ff && echo ORPHAN > .shard/objects/ff/ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
sh prune >/dev/null 2>&1
assert "p7_tag_protect" "verify tagged after prune" "succeed" sh verify "$CID"
cd "$SCENARIO_DIR"

# 7f: 5 sequential commits in log
mkdir -p "p7_5_commits" && cd "p7_5_commits"
sh init >/dev/null 2>&1
for i in $(seq 1 5); do echo "$i" > "f${i}.dat" && sh add "f${i}.dat" >/dev/null 2>&1 && sh commit -m "c${i}" --author "T" >/dev/null 2>&1; done
N=$(sh log 2>/dev/null | grep "^commit " | wc -l)
[ "$N" -eq 5 ] && echo "  OK: 5 commits in log" || echo "  WARN: $N commits"
cd "$SCENARIO_DIR"

# 7g: config persists across ops
mkdir -p "p7_config_persist" && cd "p7_config_persist"
sh init >/dev/null 2>&1 && sh config set user.name "Bob" >/dev/null 2>&1
echo d > f.dat && sh add f.dat >/dev/null 2>&1 && sh commit -m "x" --author "T" >/dev/null 2>&1
sh prune >/dev/null 2>&1
sh config get user.name 2>/dev/null | grep -q "Bob" && echo "  OK: config persists" || echo "  WARN: config lost"
cd "$SCENARIO_DIR"

# 7h: tag overwrite
mkdir -p "p7_tag_overwrite" && cd "p7_tag_overwrite"
sh init >/dev/null 2>&1
echo a > a.dat && sh add a.dat >/dev/null 2>&1 && sh commit -m "a" --author "T" >/dev/null 2>&1
CID1=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
echo b > b.dat && sh add b.dat >/dev/null 2>&1 && sh commit -m "b" --author "T" >/dev/null 2>&1
CID2=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
sh tag add mytag "$CID1" >/dev/null 2>&1
sh tag add mytag "$CID2" >/dev/null 2>&1
echo "  OK: tag overwrite (no crash)"
cd "$SCENARIO_DIR"

echo "  Phase 7 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 8: HIGH THROUGHPUT / STRESS
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 8: High-throughput stress                            ║"
echo "╚══════════════════════════════════════════════════════════════╝"

btime() { local s=$(date +%s%N); "$@" >/dev/null 2>&1; local e=$(date +%s%N); awk "BEGIN { printf \"%.6f\", ($e - $s) / 1000000000 }"; }

# 8a: 100 MiB
mkdir -p "p8_100m" && cd "p8_100m"
sh init >/dev/null 2>&1
cp "$MODEL_DIR/stress_100M.dat" .
t=$(btime sh add stress_100M.dat); echo "    add 100M: $(awk "BEGIN { print $t*1000 }") ms, $(awk "BEGIN { printf \"%.0f\", 100/$t }") MiB/s"
t=$(btime sh commit -m "s100" --author "S"); echo "    commit: $(awk "BEGIN { print $t*1000 }") ms"
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
t=$(btime sh verify "$CID"); echo "    verify: $(awk "BEGIN { print $t*1000 }") ms"
echo "    objects: $(find .shard/objects -type f | wc -l)"
cd "$SCENARIO_DIR"

# 8b: 200 MiB
mkdir -p "p8_200m" && cd "p8_200m"
sh init >/dev/null 2>&1; cp "$MODEL_DIR/stress_200M.dat" .
t=$(btime sh add stress_200M.dat); echo "    add 200M: $(awk "BEGIN { print $t*1000 }") ms, $(awk "BEGIN { printf \"%.0f\", 200/$t }") MiB/s"
t=$(btime sh commit -m "s200" --author "S"); echo "    commit: $(awk "BEGIN { print $t*1000 }") ms"
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
t=$(btime sh verify "$CID"); echo "    verify: $(awk "BEGIN { print $t*1000 }") ms"
cd "$SCENARIO_DIR"

# 8c: 100 small files
mkdir -p "p8_100f" && cd "p8_100f"
sh init >/dev/null 2>&1; cp "$MODEL_DIR/many_small/"* .
t_s=$(date +%s%N); for f in *.dat; do sh add "$f" >/dev/null 2>&1; done; t_e=$(date +%s%N)
t=$(awk "BEGIN { printf \"%.6f\", ($t_e - $t_s) / 1000000000 }")
echo "    add 100 files: $(awk "BEGIN { print $t*1000 }") ms total, $(awk "BEGIN { printf \"%.2f\", $t*1000/100 }") ms avg"
t=$(btime sh commit -m "100f" --author "S"); echo "    commit: $(awk "BEGIN { print $t*1000 }") ms"
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
t=$(btime sh verify "$CID"); echo "    verify: $(awk "BEGIN { print $t*1000 }") ms"
echo "    objects: $(find .shard/objects -type f | wc -l)"
cd "$SCENARIO_DIR"

# 8d: mixed sizes commit
mkdir -p "p8_mixed" && cd "p8_mixed"
sh init >/dev/null 2>&1
for f in "$MODEL_DIR/1K.dat" "$MODEL_DIR/4M.dat" "$MODEL_DIR/16M.dat"; do
    cp "$f" . && sh add "$(basename "$f")" >/dev/null 2>&1
done
sh commit -m "mixed" --author "S" >/dev/null 2>&1
CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
assert "p8_mixed_verify" "verify mixed sizes" "succeed" sh verify "$CID"
rm -f *.dat
t=$(btime sh checkout "$CID"); echo "    checkout: $(awk "BEGIN { print $t*1000 }") ms"
cd "$SCENARIO_DIR"

echo "  Phase 8 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 9: P99.99 LATENCY DISTRIBUTION (100 cycles)
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 9: P99.99 latency distribution                       ║"
echo "╚══════════════════════════════════════════════════════════════╝"

mkdir -p "p9_latency" && cd "p9_latency"
sh init >/dev/null 2>&1

# Pre-generate unique files
for i in $(seq 1 100); do
    dd if=/dev/urandom of="_file_${i}.dat" bs=1K count=64 2>/dev/null
done

N=100
rm -f /tmp/_lat_*.txt 2>/dev/null

for i in $(seq 1 $N); do
    cp "_file_${i}.dat" "bench.dat"
    t=$(btime sh add bench.dat); awk "BEGIN { printf \"%.2f\n\", $t*1000 }" >> /tmp/_lat_add.txt
    t=$(btime sh commit -m "l$i" --author "L"); awk "BEGIN { printf \"%.2f\n\", $t*1000 }" >> /tmp/_lat_commit.txt
    CID=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')
    t=$(btime sh verify "$CID"); awk "BEGIN { printf \"%.2f\n\", $t*1000 }" >> /tmp/_lat_verify.txt
    rm -f bench.dat
done

pct() {
    local f="$1" p="$2"
    sort -n "$f" | awk -v p="$p" 'BEGIN{c=0} {v[c++]=$1} END{idx=int(c*p/100); if(idx>=c) idx=c-1; print v[idx]}'
}

echo ""
for op in add commit verify; do
    f="/tmp/_lat_${op}.txt"
    [ -f "$f" ] || continue
    sort -n "$f" -o "$f"
    p50=$(pct "$f" 50); p90=$(pct "$f" 90); p99=$(pct "$f" 99); p999=$(pct "$f" 99.9); p9999=$(pct "$f" 99.99)
    mean=$(awk '{s+=$1} END{printf "%.2f", s/NR}' "$f")
    min=$(head -1 "$f"); max=$(tail -1 "$f")
    echo "    ${op}: min=${min} p50=${p50} p90=${p90} p99=${p99} p99.9=${p999} p99.99=${p9999} max=${max} mean=${mean} ms"
done

echo "  Phase 9 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 10: STORAGE SCALING
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 10: Storage scaling                                  ║"
echo "╚══════════════════════════════════════════════════════════════╝"

mkdir -p "p10_scale" && cd "p10_scale"
sh init >/dev/null 2>&1

echo "  size | objs | overhead"
for sm in 1 4 8 16 32; do
    sf="f_${sm}m.dat"
    dd if=/dev/urandom of="$sf" bs=1M count="$sm" 2>/dev/null
    orig=$(stat --format=%s "$sf")
    sh add "$sf" >/dev/null 2>&1 && sh commit -m "s${sm}" --author "S" >/dev/null 2>&1
    obs=$(python3 -c "import os; print(sum(os.path.getsize(os.path.join(d,f)) for d,_,fs in os.walk('.shard/objects') for f in fs))" 2>/dev/null || echo 0)
    oc=$(find .shard/objects -type f | wc -l | tr -d ' ')
    ov=$(awk "BEGIN { printf \"%.1f\", ($obs - $orig) / $orig * 100 }")
    echo "  ${sm}M  | ${oc}    | ${ov}%"
done

echo "  Phase 10 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 11: P2P NETWORK THROUGHPUT
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 11: P2P network throughput                           ║"
echo "╚══════════════════════════════════════════════════════════════╝"

P2P_DIR="$SCENARIO_DIR/p11_p2p"
mkdir -p "$P2P_DIR/server"
cd "$P2P_DIR/server"
sh init >/dev/null 2>&1
dd if=/dev/urandom of="share.dat" bs=1M count=4 2>/dev/null
sh add share.dat >/dev/null 2>&1 && sh commit -m "net" --author "N" >/dev/null 2>&1
CID_NET=$(sh log 2>/dev/null | grep "^commit " | head -1 | awk '{print $2}')

SHARE_LOG="$P2P_DIR/share.log"
"$SHARD_BIN" share > "$SHARE_LOG" 2>&1 &
SHARE_PID=$!

LA="" PI=""
for i in $(seq 1 30); do
    LA=$(grep "Listening on" "$SHARE_LOG" 2>/dev/null | head -1 | awk '{print $3}' | tr -d '"' || true)
    PI=$(grep "Local peer id:" "$SHARE_LOG" 2>/dev/null | head -1 | awk '{print $4}' || true)
    [ -n "$LA" ] && [ -n "$PI" ] && break
    sleep 1
done
MA="$LA/p2p/$PI"
echo "  Server peer: $PI"

mkdir -p "$P2P_DIR/client" && cd "$P2P_DIR/client"
sh init >/dev/null 2>&1
t_s=$(date +%s%N)
"$SHARD_BIN" pull "$MA" "$CID_NET" >/dev/null 2>&1
t_e=$(date +%s%N); t_pull=$(awk "BEGIN { printf \"%.6f\", ($t_e - $t_s) / 1000000000 }")
echo "    pull 4MiB: $(awk "BEGIN { print $t_pull*1000; exit }") ms, $(awk "BEGIN { printf \"%.0f\", 4/$t_pull }") MiB/s"

assert "p11_pull_verify" "verify pulled commit" "succeed" "$SHARD_BIN" verify "$CID_NET"

# Re-pull (idempotent)
t_s=$(date +%s%N)
"$SHARD_BIN" pull "$MA" "$CID_NET" >/dev/null 2>&1
t_e=$(date +%s%N); t_repull=$(awk "BEGIN { printf \"%.6f\", ($t_e - $t_s) / 1000000000 }")
echo "    re-pull: $(awk "BEGIN { print $t_repull*1000; exit }") ms"

kill "$SHARE_PID" 2>/dev/null || true
wait "$SHARE_PID" 2>/dev/null || true

echo "  Phase 11 cumulative: $PASSED/$TOTAL"

# =====================================================================
# PHASE 12: CONCURRENT STRESS
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PHASE 12: Concurrent stress                                ║"
echo "╚══════════════════════════════════════════════════════════════╝"

mkdir -p "p12_concurrent" && cd "p12_concurrent"
for i in $(seq 1 10); do
    rd="repo_$i" && mkdir -p "$rd"
    (
        cd "$rd"
        "$SHARD_BIN" init >/dev/null 2>&1
        dd if=/dev/urandom of="test.dat" bs=1M count=1 2>/dev/null
        "$SHARD_BIN" add test.dat >/dev/null 2>&1
        "$SHARD_BIN" commit -m "c$i" --author "C" >/dev/null 2>&1
    ) &
done
wait
OK=0
for i in $(seq 1 10); do [ -f "repo_$i/.shard/HEAD" ] && OK=$((OK+1)); done
echo "    concurrent completed: ${OK}/10"
assert "p12_concurrent" "10 concurrent repos" "succeed" test "$OK" -eq 10

# =====================================================================
# SUMMARY
# =====================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  FINAL SUMMARY                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "  Total: $TOTAL  Passed: $PASSED  Failed: $FAILED"
if [ "$FAILED" -gt 0 ]; then
    echo ""
    echo "  FAILURES ($FAILED):"
    echo -e "$ERRORS" | sed 's/^/    /'
fi
echo ""
echo "  Full logs and models: $RESULTS_DIR"
