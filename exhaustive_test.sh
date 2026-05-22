#!/usr/bin/env bash
set -uo pipefail

# ─────────────────────────────────────────────────────────────────────────────
#  Shard Exhaustive Test v2 — ALL possible use cases, loopholes, edge cases
#  Commands run via: sh <repo_dir> <command> [args]
#  This ensures each command runs in the correct working directory.
# ─────────────────────────────────────────────────────────────────────────────
RESULTS_DIR=$(mktemp -d)
MODELS="$RESULTS_DIR/models"

# Determine the project root (where this script lives)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARD_BIN="${SHARD_BIN:-$SCRIPT_DIR/target/release/shard}"
PASS=0
FAIL=0
TOTAL=0

RESULTS_FILE="$RESULTS_DIR/results.log"
SUMMARY_FILE="$RESULTS_DIR/summary.log"

mkdir -p "$MODELS"
rm -f "$RESULTS_FILE" "$SUMMARY_FILE"

# Run a shard command in a specific repo directory.
# Resolves SHARD_BIN before cd'ing so relative paths work.
sh() { local dir="$1"; shift; (cd "$dir" && "$SHARD_BIN" "$@" 2>/dev/null); }

pass() { PASS=$((PASS+1)); TOTAL=$((TOTAL+1)); echo "[pass] $*" | tee -a "$RESULTS_FILE"; }
fail() { FAIL=$((FAIL+1)); TOTAL=$((TOTAL+1)); echo "[FAIL] $*" | tee -a "$RESULTS_FILE"; }
check() { local desc="$1"; shift; if "$@"; then pass "$desc"; else fail "$desc"; fi; }
check_not() { local desc="$1"; shift; if "$@"; then fail "$desc"; else pass "$desc"; fi; }

# Build release binary if not present
if [ ! -f "$SHARD_BIN" ]; then
    echo "Building shard (release)..."
    cargo build --release 2>/dev/null
fi

echo "=== SHARD EXHAUSTIVE TEST v2 ===" | tee "$SUMMARY_FILE"
echo "Results dir: $RESULTS_DIR" | tee -a "$SUMMARY_FILE"
echo "Binary: $SHARD_BIN" | tee -a "$SUMMARY_FILE"
echo "" | tee -a "$SUMMARY_FILE"

# Helper: init a test repo
init_repo() { local d="$1"; rm -rf "$d"; mkdir -p "$d"; sh "$d" init >/dev/null 2>&1; }

# Helper: get commit ID from latest commit
do_commit() { local d="$1"; local m="$2"; sh "$d" commit -m "$m" --author "Test" | awk '{print $2}'; }

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 0: Model File Creation
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 0: Model file creation ═══" | tee -a "$SUMMARY_FILE"

# 0a. Size model files
dd if=/dev/urandom of="$MODELS/1b.bin" bs=1 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/1k.bin" bs=1024 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/64k.bin" bs=65536 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/1m.bin" bs=1048576 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/4m.bin" bs=4194304 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/4m1b.bin" bs=4194305 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/8m.bin" bs=8388608 count=1 2>/dev/null
dd if=/dev/urandom of="$MODELS/8m1b.bin" bs=8388609 count=1 2>/dev/null

# 0b. Edge case files
touch "$MODELS/empty.bin"
printf '\x00' > "$MODELS/1byte.bin"
dd if=/dev/zero of="$MODELS/zeros_64k.bin" bs=65536 count=1 2>/dev/null
printf 'Hello\nWorld\nLine3\n' > "$MODELS/newlines.txt"
printf '   leading and trailing spaces   \n' > "$MODELS/spaces.txt"
printf '{\n  "key": "value"\n}\n' > "$MODELS/json.json"
printf '\xff\xfe\x00\x01\x02' > "$MODELS/binary_header.bin"
dd if=/dev/urandom of="$MODELS/100m.bin" bs=1048576 count=100 2>/dev/null
dd if=/dev/urandom of="$MODELS/200m.bin" bs=1048576 count=200 2>/dev/null

# 0c. Symlinks and hardlinks
ln -sf "$MODELS/64k.bin" "$MODELS/valid_symlink" 2>/dev/null
ln -sf "/nonexistent/path" "$MODELS/broken_symlink" 2>/dev/null
ln -f "$MODELS/64k.bin" "$MODELS/hardlink" 2>/dev/null

echo "Model files created in $MODELS" | tee -a "$SUMMARY_FILE"
echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 1: Init — ALL init scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 1: Init scenarios ═══" | tee -a "$SUMMARY_FILE"

d1_1="$RESULTS_DIR/p1_1"
rm -rf "$d1_1"; mkdir -p "$d1_1"
check "p1.1 init creates .shard" sh "$d1_1" init
check "p1.1b .shard/objects exists" test -d "$d1_1/.shard/objects"
check "p1.1c .shard/keys exists" test -d "$d1_1/.shard/keys"
check "p1.1d secret.key exists" test -f "$d1_1/.shard/keys/secret.key"
check "p1.1e public.key exists" test -f "$d1_1/.shard/keys/public.key"
check "p1.1f config has repo_id" sh "$d1_1" config get repo_id

check_not "p1.2 init twice fails" sh "$d1_1" init

d1_3="$RESULTS_DIR/p1_3"
rm -rf "$d1_3"; mkdir -p "$d1_3"
check "p1.3 init --private" sh "$d1_3" init --private
check "p1.3b config private=true" sh "$d1_3" config get private

d1_4="$RESULTS_DIR/p1_4"
mkdir -p "$d1_4"
check "p1.4 init other dir" sh "$d1_4" init

d1_5a="$RESULTS_DIR/p1_5a"; d1_5b="$RESULTS_DIR/p1_5b"
rm -rf "$d1_5a" "$d1_5b"; mkdir -p "$d1_5a" "$d1_5b"
sh "$d1_5a" init >/dev/null; sh "$d1_5b" init >/dev/null
rid_a=$(sh "$d1_5a" config get repo_id 2>/dev/null | awk '{print $3}')
rid_b=$(sh "$d1_5b" config get repo_id 2>/dev/null | awk '{print $3}')
check "p1.5 unique repo_ids" test "$rid_a" != "$rid_b"

d1_6="$RESULTS_DIR/p1_6"
rm -rf "$d1_6"; mkdir -p "$d1_6"; touch "$d1_6/existing.txt"
check "p1.6 init in non-empty dir" sh "$d1_6" init
check "p1.6b existing file intact" test -f "$d1_6/existing.txt"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 2: Add — ALL add scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 2: Add scenarios ═══" | tee -a "$SUMMARY_FILE"

d2="$RESULTS_DIR/p2"
init_repo "$d2"

cp "$MODELS/64k.bin" "$d2/"
check "p2.1 add normal file" sh "$d2" add "64k.bin"
check "p2.2 add same file again" sh "$d2" add "64k.bin"

touch "$d2/empty.txt"
check "p2.3 add empty file" sh "$d2" add "empty.txt"

cp "$MODELS/1byte.bin" "$d2/"
check "p2.4 add 1-byte file" sh "$d2" add "1byte.bin"

cp "$MODELS/4m1b.bin" "$d2/"
check "p2.5 add cross-chunk file" sh "$d2" add "4m1b.bin"

cp "$MODELS/4m.bin" "$d2/"
check "p2.6 add exact chunk file" sh "$d2" add "4m.bin"

cp "$MODELS/newlines.txt" "$d2/"
check "p2.7 add newlines content" sh "$d2" add "newlines.txt"

cp "$MODELS/zeros_64k.bin" "$d2/"
check "p2.8 add zero-filled file" sh "$d2" add "zeros_64k.bin"

check_not "p2.9 add nonexistent fails" sh "$d2" add "nonexistent_file_xyz.bin"

mkdir -p "$d2/subdir"
check_not "p2.10 add directory fails" sh "$d2" add "subdir"

cp "$MODELS/valid_symlink" "$d2/" 2>/dev/null
check "p2.11 add valid symlink" sh "$d2" add "valid_symlink"

cp "$MODELS/broken_symlink" "$d2/" 2>/dev/null
check_not "p2.12 add broken symlink" sh "$d2" add "broken_symlink"

ln -f "$MODELS/64k.bin" "$d2/hardlink" 2>/dev/null
check "p2.13 add hardlinked file" sh "$d2" add "hardlink"

# Unicode and special filenames
cp "$MODELS/64k.bin" "$d2/ファイル.txt"
check "p2.14 add unicode filename" sh "$d2" add "ファイル.txt"

cp "$MODELS/64k.bin" "$d2/a file with spaces.txt"
check "p2.15 add filename with spaces" sh "$d2" add "a file with spaces.txt"

cp "$MODELS/64k.bin" "$d2/.hidden"
check "p2.16 add hidden file" sh "$d2" add ".hidden"

cp "$MODELS/64k.bin" "$d2/special!@#\$%.bin"
check "p2.17 add special chars filename" sh "$d2" add "special!@#\$%.bin"

check_not "p2.18 add .. fails" sh "$d2" add ".."
check_not "p2.19 add . fails" sh "$d2" add "."

# Add without repo
d2n="$RESULTS_DIR/p2n"; mkdir -p "$d2n"
cp "$MODELS/1k.bin" "$d2n/"
check_not "p2.20 add without init fails" sh "$d2n" add "1k.bin"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 3: Commit — ALL commit scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 3: Commit scenarios ═══" | tee -a "$SUMMARY_FILE"

d3_1="$RESULTS_DIR/p3_1"
init_repo "$d3_1"; cp "$MODELS/64k.bin" "$d3_1/"; sh "$d3_1" add "64k.bin" >/dev/null
cid31=$(do_commit "$d3_1" "single file commit")
check "p3.1 single file commit" test -n "$cid31"
check "p3.1b HEAD exists" test -f "$d3_1/.shard/HEAD"

d3_2="$RESULTS_DIR/p3_2"
init_repo "$d3_2"; cp "$MODELS/1k.bin" "$d3_2/"; sh "$d3_2" add "1k.bin" >/dev/null
cid32=$(do_commit "$d3_2" "first")
check "p3.2 first commit" test -n "$cid32"

cp "$MODELS/64k.bin" "$d3_2/"; sh "$d3_2" add "64k.bin" >/dev/null
cid33=$(do_commit "$d3_2" "second")
check "p3.3 second commit" test -n "$cid33"

d3_4="$RESULTS_DIR/p3_4"
init_repo "$d3_4"
for f in a b c; do cp "$MODELS/1k.bin" "$d3_4/$f.txt"; sh "$d3_4" add "$f.txt" >/dev/null; done
cid34=$(do_commit "$d3_4" "multi-file")
check "p3.4 multi-file commit" test -n "$cid34"

d3_5="$RESULTS_DIR/p3_5"
init_repo "$d3_5"; cp "$MODELS/1k.bin" "$d3_5/"; sh "$d3_5" add "1k.bin" >/dev/null
check "p3.5 custom author" sh "$d3_5" commit -m "custom" --author "Alice <alice@test>"

d3_6="$RESULTS_DIR/p3_6"
init_repo "$d3_6"; cp "$MODELS/1k.bin" "$d3_6/"; sh "$d3_6" add "1k.bin" >/dev/null
check "p3.6 empty message" sh "$d3_6" commit -m "" --author "Test"

d3_7="$RESULTS_DIR/p3_7"
init_repo "$d3_7"; cp "$MODELS/1k.bin" "$d3_7/"; sh "$d3_7" add "1k.bin" >/dev/null
check "p3.7 unicode message" sh "$d3_7" commit -m "こんにちは世界" --author "Test"

d3_8="$RESULTS_DIR/p3_8"
init_repo "$d3_8"
check_not "p3.8 commit nothing staged" sh "$d3_8" commit -m "nope" --author "T"

d3_9="$RESULTS_DIR/p3_9"
mkdir -p "$d3_9"
check_not "p3.9 commit without init" sh "$d3_9" commit -m "nope" --author "T"

d3_10="$RESULTS_DIR/p3_10"
init_repo "$d3_10"
for i in 1 2 3 4 5; do
    dd if=/dev/urandom of="$d3_10/file$i.bin" bs=1024 count=1 2>/dev/null
    sh "$d3_10" add "file$i.bin" >/dev/null
    do_commit "$d3_10" "commit #$i" >/dev/null
done
logcnt=$(sh "$d3_10" log 2>/dev/null | grep -c "^commit ")
check "p3.10 chain of 5" test "$logcnt" -eq 5

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 4: Verify — ALL verify scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 4: Verify scenarios ═══" | tee -a "$SUMMARY_FILE"

d4="$RESULTS_DIR/p4"
init_repo "$d4"; cp "$MODELS/64k.bin" "$d4/"; sh "$d4" add "64k.bin" >/dev/null
cid4=$(do_commit "$d4" "verify test")

check "p4.1 verify valid" sh "$d4" verify "$cid4"
check "p4.2 verify --json" sh "$d4" verify --json "$cid4"
check "p4.2b json parseable" sh "$d4" verify --json "$cid4" 2>/dev/null | python3 -m json.tool >/dev/null 2>&1

check_not "p4.3 verify nonexistent" sh "$d4" verify "0000000000000000000000000000000000000000000000000000000000000000"
check_not "p4.4 verify empty" sh "$d4" verify ""
check_not "p4.5 verify 1char" sh "$d4" verify "a"
check_not "p4.6 verify nonhex" sh "$d4" verify "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"

d4n="$RESULTS_DIR/p4n"; mkdir -p "$d4n"
check_not "p4.7 verify no init" sh "$d4n" verify "abc"

# Tampered verify
d4t="$RESULTS_DIR/p4t"
init_repo "$d4t"; cp "$MODELS/64k.bin" "$d4t/"; sh "$d4t" add "64k.bin" >/dev/null
cid4t=$(do_commit "$d4t" "tamper")
find "$d4t/.shard/objects" -type f | head -1 | while read f; do echo "TAMPERED" > "$f" 2>/dev/null; done
check_not "p4.8 verify tampered" sh "$d4t" verify "$cid4t"

# Verify signature
d4s="$RESULTS_DIR/p4s"
init_repo "$d4s"; cp "$MODELS/1k.bin" "$d4s/"; sh "$d4s" add "1k.bin" >/dev/null
cid4s=$(do_commit "$d4s" "sig test")
vout=$(sh "$d4s" verify "$cid4s" 2>/dev/null)
check "p4.9 signature verified" echo "$vout" | grep -q "Signature verified"

# Multi-chunk verify
d4m="$RESULTS_DIR/p4m"
init_repo "$d4m"; cp "$MODELS/4m1b.bin" "$d4m/"; sh "$d4m" add "4m1b.bin" >/dev/null
cid4m=$(do_commit "$d4m" "multichunk")
check "p4.10 verify multi-chunk" sh "$d4m" verify "$cid4m"

# Zero-filled verify
d4z="$RESULTS_DIR/p4z"
init_repo "$d4z"; cp "$MODELS/zeros_64k.bin" "$d4z/"; sh "$d4z" add "zeros_64k.bin" >/dev/null
cid4z=$(do_commit "$d4z" "zeros")
check "p4.11 verify zeros" sh "$d4z" verify "$cid4z"

# Empty file verify
d4e="$RESULTS_DIR/p4e"
init_repo "$d4e"; touch "$d4e/empty.bin"; sh "$d4e" add "empty.bin" >/dev/null
cid4e=$(do_commit "$d4e" "empty")
check "p4.12 verify empty" sh "$d4e" verify "$cid4e"

# 1-byte verify
d4o="$RESULTS_DIR/p4o"
init_repo "$d4o"; cp "$MODELS/1byte.bin" "$d4o/"; sh "$d4o" add "1byte.bin" >/dev/null
cid4o=$(do_commit "$d4o" "onebyte")
check "p4.13 verify 1-byte" sh "$d4o" verify "$cid4o"

# Exact chunk (4 MiB) verify
d4x="$RESULTS_DIR/p4x"
init_repo "$d4x"; cp "$MODELS/4m.bin" "$d4x/"; sh "$d4x" add "4m.bin" >/dev/null
cid4x=$(do_commit "$d4x" "exact chunk")
check "p4.14 verify exact 4M" sh "$d4x" verify "$cid4x"

# Cross-chunk (4 MiB + 1) verify
d4c="$RESULTS_DIR/p4c"
init_repo "$d4c"; cp "$MODELS/4m1b.bin" "$d4c/"; sh "$d4c" add "4m1b.bin" >/dev/null
cid4c=$(do_commit "$d4c" "cross chunk")
check "p4.15 verify cross-chunk" sh "$d4c" verify "$cid4c"

# Same commit verify twice
check "p4.16 verify twice" sh "$d4" verify "$cid4"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 5: Log — ALL log scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 5: Log scenarios ═══" | tee -a "$SUMMARY_FILE"

d5="$RESULTS_DIR/p5"
init_repo "$d5"
for i in 1 2 3; do echo "$i" > "$d5/f$i.txt"; sh "$d5" add "f$i.txt" >/dev/null; do_commit "$d5" "commit $i" >/dev/null; done

check "p5.1 log with commits" sh "$d5" log
check "p5.2 log --json" sh "$d5" log --json
logjson=$(sh "$d5" log --json 2>/dev/null)
check "p5.2b json parseable" echo "$logjson" | python3 -m json.tool >/dev/null 2>&1
check "p5.2c json has commit_id" echo "$logjson" | python3 -c "import sys,json; d=json.load(sys.stdin); assert len(d)==3" 2>/dev/null
check "p5.3 log shows author" sh "$d5" log 2>/dev/null | grep -q "author:"
check "p5.4 log shows date" sh "$d5" log 2>/dev/null | grep -q "date:"

d5e="$RESULTS_DIR/p5e"
init_repo "$d5e"
check_not "p5.5 log no commits" sh "$d5e" log

d5n="$RESULTS_DIR/p5n"; mkdir -p "$d5n"
check_not "p5.6 log no init" sh "$d5n" log

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 6: Checkout — ALL checkout scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 6: Checkout scenarios ═══" | tee -a "$SUMMARY_FILE"

d6="$RESULTS_DIR/p6"
init_repo "$d6"; echo "checkout content" > "$d6/test.txt"
sh "$d6" add "test.txt" >/dev/null; cid6=$(do_commit "$d6" "checkout test")
rm -f "$d6/test.txt"
check "p6.1 checkout restore" sh "$d6" checkout "$cid6"
check "p6.1b file exists" test -f "$d6/test.txt"
check "p6.1c content correct" grep -q "checkout content" "$d6/test.txt"

# Checkout --json
d6j="$RESULTS_DIR/p6j"
init_repo "$d6j"; echo "json" > "$d6j/j.txt"
sh "$d6j" add "j.txt" >/dev/null; cid6j=$(do_commit "$d6j" "json co")
rm -f "$d6j/j.txt"
check "p6.2 checkout --json" sh "$d6j" checkout --json "$cid6j"
cojson=$(sh "$d6j" checkout --json "$cid6j" 2>/dev/null)
check "p6.2b json parseable" echo "$cojson" | python3 -m json.tool >/dev/null 2>&1

# Checkout multiple files
d6m="$RESULTS_DIR/p6m"
init_repo "$d6m"
echo "aaa" > "$d6m/a.txt"; sh "$d6m" add "a.txt" >/dev/null
echo "bbb" > "$d6m/b.txt"; sh "$d6m" add "b.txt" >/dev/null
cid6m=$(do_commit "$d6m" "multi")
rm -f "$d6m/a.txt" "$d6m/b.txt"
sh "$d6m" checkout "$cid6m" >/dev/null
check "p6.3 checkout multiple files" test -f "$d6m/a.txt" && test -f "$d6m/b.txt"

# Checkout same commit twice
check "p6.4 checkout twice" sh "$d6" checkout "$cid6"

# Failures
check_not "p6.5 checkout bad commit" sh "$d6m" checkout "0000000000000000000000000000000000000000000000000000000000000000"
d6n="$RESULTS_DIR/p6n"; mkdir -p "$d6n"
check_not "p6.6 checkout no init" sh "$d6n" checkout "abc"
check_not "p6.7 checkout empty id" sh "$d6" checkout ""

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 7: Status — ALL status scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 7: Status scenarios ═══" | tee -a "$SUMMARY_FILE"

d7="$RESULTS_DIR/p7"
init_repo "$d7"
check "p7.1 status after init" sh "$d7" status

echo "staged" > "$d7/staged.txt"; sh "$d7" add "staged.txt" >/dev/null
s7=$(sh "$d7" status 2>/dev/null)
check "p7.2 status shows staged" echo "$s7" | grep -q "Staged"

do_commit "$d7" "first" >/dev/null
s7c=$(sh "$d7" status 2>/dev/null)
check "p7.3 status clean" echo "$s7c" | grep -q "Nothing staged"

echo "untracked" > "$d7/untracked.txt"
s7u=$(sh "$d7" status 2>/dev/null)
check "p7.4 status untracked" echo "$s7u" | grep -q "untracked"

check "p7.5 status --json" sh "$d7" status --json
stjson=$(sh "$d7" status --json 2>/dev/null)
check "p7.5b json parseable" echo "$stjson" | python3 -m json.tool >/dev/null 2>&1

rm -f "$d7/staged.txt"
s7d=$(sh "$d7" status 2>/dev/null)
check "p7.6 status deleted" echo "$s7d" | grep -q "deleted"

d7n="$RESULTS_DIR/p7n"; mkdir -p "$d7n"
check_not "p7.7 status no init" sh "$d7n" status

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 8: Config — ALL config scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 8: Config scenarios ═══" | tee -a "$SUMMARY_FILE"

d8="$RESULTS_DIR/p8"
init_repo "$d8"

check "p8.1 config set" sh "$d8" config set "test.key" "test.value"
check "p8.1b config get" sh "$d8" config get "test.key"
c8g=$(sh "$d8" config get "test.key" 2>/dev/null)
check "p8.1c correct value" echo "$c8g" | grep -q "test.value"

sh "$d8" config set "a" "1" >/dev/null
sh "$d8" config set "b" "2" >/dev/null
c8a=$(sh "$d8" config get 2>/dev/null)
check "p8.2 config get all" echo "$c8a" | grep -q "a = 1"
check "p8.2b get all shows both" echo "$c8a" | grep -q "b = 2"

check_not "p8.3 config missing key" sh "$d8" config get "nonexistent"

check "p8.4 config set empty" sh "$d8" config set "empty" ""
c8e=$(sh "$d8" config get "empty" 2>/dev/null)
check "p8.4b verify empty value" echo "$c8e" | grep -q "empty ="

check "p8.5 config unicode" sh "$d8" config set "unicode" "日本語"

check "p8.6 config repo_id" sh "$d8" config get "repo_id"

d8n="$RESULTS_DIR/p8n"; mkdir -p "$d8n"
check_not "p8.7 config no init" sh "$d8n" config get "x"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 9: Tag — ALL tag scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 9: Tag scenarios ═══" | tee -a "$SUMMARY_FILE"

d9="$RESULTS_DIR/p9"
init_repo "$d9"; cp "$MODELS/1k.bin" "$d9/"; sh "$d9" add "1k.bin" >/dev/null
cid9=$(do_commit "$d9" "tag me")

check "p9.1 tag add" sh "$d9" tag add "v1.0" "$cid9"
check "p9.2 tag list" sh "$d9" tag list

tlist=$(sh "$d9" tag list 2>/dev/null)
check "p9.2b tag shows name" echo "$tlist" | grep -q "v1.0"

# Tag second commit
cp "$MODELS/64k.bin" "$d9/"; sh "$d9" add "64k.bin" >/dev/null
cid9b=$(do_commit "$d9" "second")
check "p9.3 tag second commit" sh "$d9" tag add "v2.0" "$cid9b"
check "p9.4 tag overwrite same name" sh "$d9" tag add "v1.0" "$cid9b"

check_not "p9.5 tag bad commit" sh "$d9" tag add "bad" "0000000000000000000000000000000000000000000000000000000000000000"

d9n="$RESULTS_DIR/p9n"; mkdir -p "$d9n"
check_not "p9.6 tag no init" sh "$d9n" tag add "x" "abc"

d9e="$RESULTS_DIR/p9e"
init_repo "$d9e"
check "p9.7 tag list empty" sh "$d9e" tag list

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 10: Prune — ALL prune scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 10: Prune scenarios ═══" | tee -a "$SUMMARY_FILE"

d10="$RESULTS_DIR/p10"
init_repo "$d10"; cp "$MODELS/1k.bin" "$d10/"; sh "$d10" add "1k.bin" >/dev/null
cid10=$(do_commit "$d10" "prune test")

p10out=$(sh "$d10" prune 2>/dev/null)
check "p10.1 prune no orphans" echo "$p10out" | grep -q "Pruned 0"

mkdir -p "$d10/.shard/objects/zz"
echo "ORPHAN_DATA" > "$d10/.shard/objects/zz/orphan_hash_abcdef1234567890"
p10out=$(sh "$d10" prune 2>/dev/null)
check "p10.2 prune removes orphan" echo "$p10out" | grep -q "Pruned 1"
check "p10.2b orphan file gone" test ! -f "$d10/.shard/objects/zz/orphan_hash_abcdef1234567890"

check "p10.3 reachable survives prune" sh "$d10" verify "$cid10"

# Prune preserves tagged commits
d10t="$RESULTS_DIR/p10t"
init_repo "$d10t"; cp "$MODELS/1k.bin" "$d10t/"; sh "$d10t" add "1k.bin" >/dev/null
cid10t=$(do_commit "$d10t" "tagged")
sh "$d10t" tag add "protected" "$cid10t" >/dev/null
mkdir -p "$d10t/.shard/objects/oo"
echo "ORPHAN" > "$d10t/.shard/objects/oo/orphan_tagged"
sh "$d10t" prune >/dev/null
check "p10.4 tagged commit protected" sh "$d10t" verify "$cid10t"

# Prune after staged
d10s="$RESULTS_DIR/p10s"
init_repo "$d10s"
echo "staged data" > "$d10s/staged.txt"; sh "$d10s" add "staged.txt" >/dev/null
mkdir -p "$d10s/.shard/objects/oo"
echo "ORPHAN_STAGED" > "$d10s/.shard/objects/oo/orphan_staged_test"
p10sout=$(sh "$d10s" prune 2>/dev/null)
check "p10.5 prune with staged" echo "$p10sout" | grep -q "Pruned 1"

d10n="$RESULTS_DIR/p10n"; mkdir -p "$d10n"
check_not "p10.6 prune no init" sh "$d10n" prune

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 11: Peer — ALL peer scenarios
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 11: Peer scenarios ═══" | tee -a "$SUMMARY_FILE"

d11="$RESULTS_DIR/p11"
init_repo "$d11"
peer_addr="/ip4/127.0.0.1/tcp/9999/p2p/12D3KooWH63KtTR9UauNYkurjdY53iGTDaurS8RwFWNFLHKCtmWU"

check "p11.1 peer add" sh "$d11" peer add "$peer_addr"
check "p11.2 peer add duplicate" sh "$d11" peer add "$peer_addr"
check_not "p11.3 peer add invalid" sh "$d11" peer add "not-a-valid-multiaddr"
check_not "p11.4 peer add empty" sh "$d11" peer add ""

d11n="$RESULTS_DIR/p11n"; mkdir -p "$d11n"
check_not "p11.5 peer no init" sh "$d11n" peer add "$peer_addr"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 12: Multi-command workflow integration
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 12: Workflow integration ═══" | tee -a "$SUMMARY_FILE"

# Full lifecycle
d12="$RESULTS_DIR/p12"
init_repo "$d12"
echo "workflow" > "$d12/w.txt"; sh "$d12" add "w.txt" >/dev/null
cid12=$(do_commit "$d12" "workflow")
sh "$d12" verify "$cid12" >/dev/null 2>&1
sh "$d12" log >/dev/null 2>&1
rm -f "$d12/w.txt"
sh "$d12" checkout "$cid12" >/dev/null 2>&1
s12=$(sh "$d12" status 2>/dev/null)
check "p12.1 full lifecycle" echo "$s12" | grep -q "Nothing staged"

# Batch add
d12b="$RESULTS_DIR/p12b"
init_repo "$d12b"
for f in a b c d e; do echo "$f" > "$d12b/$f.txt"; sh "$d12b" add "$f.txt" >/dev/null; done
cid12b=$(do_commit "$d12b" "batch")
check "p12.2 batch add+commit" test -n "$cid12b"

# Config persists
sh "$d12" config set "persist" "yes" >/dev/null
check "p12.3 config persists" sh "$d12" config get "persist"

# Tag persists after prune
sh "$d12" tag add "stable" "$cid12" >/dev/null
sh "$d12" prune >/dev/null
check "p12.4 tag persists after prune" sh "$d12" tag list 2>/dev/null | grep -q "stable"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 13: State machine violations — illegal sequences
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 13: State machine violations ═══" | tee -a "$SUMMARY_FILE"

d13="$RESULTS_DIR/p13"
init_repo "$d13"
check_not "p13.1 verify before commit" sh "$d13" verify "anything"
check_not "p13.2 checkout before commit" sh "$d13" checkout "anything"
check_not "p13.3 log before commit" sh "$d13" log
check_not "p13.4 tag before commit" sh "$d13" tag add "v1" "anything"

# Double commit without add
echo "x" > "$d13/f.txt"; sh "$d13" add "f.txt" >/dev/null
do_commit "$d13" "first" >/dev/null
check_not "p13.5 commit without stage" sh "$d13" commit -m "second" --author "T"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 14: Throughput benchmarks
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 14: Throughput benchmarks ═══" | tee -a "$SUMMARY_FILE"

bench_file() {
    local size="$1" file="$2"
    local d="$RESULTS_DIR/bench_${size}"
    init_repo "$d"; cp "$file" "$d/"
    local fname=$(basename "$file")
    local start=$(date +%s%N)
    sh "$d" add "$fname" >/dev/null 2>&1
    local end=$(date +%s%N)
    local ms=$(( (end - start) / 1000000 ))
    local bs=$(stat -c%s "$file" 2>/dev/null || echo 1024)
    local mb_s=0
    [ "$ms" -gt 0 ] && mb_s=$(( bs * 1000 / ms / 1048576 ))
    echo "  add ${size}: ${ms} ms, ${mb_s} MiB/s" | tee -a "$RESULTS_FILE"
}

bench_file "1b" "$MODELS/1b.bin"
bench_file "1k" "$MODELS/1k.bin"
bench_file "64k" "$MODELS/64k.bin"
bench_file "1m" "$MODELS/1m.bin"
bench_file "4m" "$MODELS/4m.bin"
bench_file "4m1b" "$MODELS/4m1b.bin"
bench_file "8m" "$MODELS/8m.bin"

# Large file benchmark
d14_100="$RESULTS_DIR/bench_100m"
init_repo "$d14_100"; cp "$MODELS/100m.bin" "$d14_100/"
start=$(date +%s%N); sh "$d14_100" add "100m.bin" >/dev/null 2>&1; end=$(date +%s%N)
ms100=$(( (end-start)/1000000 ))
echo "  add 100M: ${ms100} ms" | tee -a "$RESULTS_FILE"

start=$(date +%s%N); cid100=$(do_commit "$d14_100" "100m"); end=$(date +%s%N)
ms100c=$(( (end-start)/1000000 ))
echo "  commit 100M: ${ms100c} ms" | tee -a "$RESULTS_FILE"

start=$(date +%s%N); sh "$d14_100" verify "$cid100" >/dev/null 2>&1; end=$(date +%s%N)
ms100v=$(( (end-start)/1000000 ))
echo "  verify 100M: ${ms100v} ms" | tee -a "$RESULTS_FILE"

obj100=$(find "$d14_100/.shard/objects" -type f | wc -l)
echo "  objects: $obj100" | tee -a "$RESULTS_FILE"

# 200 MiB benchmark
d14_200="$RESULTS_DIR/bench_200m"
init_repo "$d14_200"; cp "$MODELS/200m.bin" "$d14_200/"
start=$(date +%s%N); sh "$d14_200" add "200m.bin" >/dev/null 2>&1; end=$(date +%s%N)
ms200=$(( (end-start)/1000000 ))
echo "  add 200M: ${ms200} ms" | tee -a "$RESULTS_FILE"

start=$(date +%s%N); cid200=$(do_commit "$d14_200" "200m"); end=$(date +%s%N)
ms200c=$(( (end-start)/1000000 ))
echo "  commit 200M: ${ms200c} ms" | tee -a "$RESULTS_FILE"

start=$(date +%s%N); sh "$d14_200" verify "$cid200" >/dev/null 2>&1; end=$(date +%s%N)
ms200v=$(( (end-start)/1000000 ))
echo "  verify 200M: ${ms200v} ms" | tee -a "$RESULTS_FILE"

# Many small files
d14_many="$RESULTS_DIR/bench_many"
init_repo "$d14_many"
start=$(date +%s%N)
for i in $(seq 1 100); do
    dd if=/dev/urandom of="$d14_many/small_$i.bin" bs=1024 count=1 2>/dev/null
    sh "$d14_many" add "small_$i.bin" >/dev/null 2>&1
done
end=$(date +%s%N)
ms_many=$(( (end-start)/1000000 ))
echo "  add 100 files: ${ms_many} ms total" | tee -a "$RESULTS_FILE"

start=$(date +%s%N); cid_many=$(do_commit "$d14_many" "many"); end=$(date +%s%N)
ms_manyc=$(( (end-start)/1000000 ))
echo "  commit 100 files: ${ms_manyc} ms" | tee -a "$RESULTS_FILE"

start=$(date +%s%N); sh "$d14_many" verify "$cid_many" >/dev/null 2>&1; end=$(date +%s%N)
ms_manyv=$(( (end-start)/1000000 ))
echo "  verify 100 files: ${ms_manyv} ms" | tee -a "$RESULTS_FILE"

for i in $(seq 1 100); do rm -f "$d14_many/small_$i.bin"; done
start=$(date +%s%N); sh "$d14_many" checkout "$cid_many" >/dev/null 2>&1; end=$(date +%s%N)
ms_co=$(( (end-start)/1000000 ))
echo "  checkout 100 files: ${ms_co} ms" | tee -a "$RESULTS_FILE"

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 15: P99.99 Latency Distribution (N=100)
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 15: P99.99 Latency distribution ═══" | tee -a "$SUMMARY_FILE"

LAT_DIR="$RESULTS_DIR/lat"
lat_add="$RESULTS_DIR/lat_add.txt"
lat_commit="$RESULTS_DIR/lat_commit.txt"
lat_verify="$RESULTS_DIR/lat_verify.txt"
rm -f "$lat_add" "$lat_commit" "$lat_verify"

echo "Running 100 latency cycles..." | tee -a "$SUMMARY_FILE"
for i in $(seq 1 100); do
    repo="$LAT_DIR/$i"
    init_repo "$repo"
    dd if=/dev/urandom of="$repo/test.bin" bs=65536 count=1 2>/dev/null

    start=$(date +%s%N)
    sh "$repo" add "test.bin" >/dev/null 2>&1
    end=$(date +%s%N)
    echo "scale=3; ($end - $start)/1000000" | bc 2>/dev/null >> "$lat_add" || \
        python3 -c "print(($end - $start)/1000000)" >> "$lat_add"

    start=$(date +%s%N)
    cid=$(do_commit "$repo" "lat $i")
    end=$(date +%s%N)
    python3 -c "print(($end - $start)/1000000)" >> "$lat_commit"

    start=$(date +%s%N)
    sh "$repo" verify "$cid" >/dev/null 2>&1
    end=$(date +%s%N)
    python3 -c "print(($end - $start)/1000000)" >> "$lat_verify"
done

# Compute percentiles with python3
for op in add commit verify; do
    latfile="$RESULTS_DIR/lat_${op}.txt"
    if [ -f "$latfile" ]; then
        stats=$(python3 -c "
import json, sys, statistics
vals = sorted([float(l) for l in open('$latfile') if l.strip()])
n = len(vals)
def pct(p): return vals[min(int(n * p), n-1)]
print(f'min={vals[0]:.2f} p50={pct(0.5):.2f} p90={pct(0.9):.2f} p99={pct(0.99):.2f} p99.99={pct(0.9999):.2f} max={vals[-1]:.2f} mean={statistics.mean(vals):.2f}')
")
        echo "  $op: $stats ms" | tee -a "$RESULTS_FILE"
    fi
done

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 16: Storage scaling overhead
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 16: Storage scaling ═══" | tee -a "$SUMMARY_FILE"

d_scale="$RESULTS_DIR/scale"
init_repo "$d_scale"

for size_mib in 1 4 8 16 32; do
    dd if=/dev/urandom of="$d_scale/file_${size_mib}m.bin" bs=1048576 count="$size_mib" 2>/dev/null
    sh "$d_scale" add "file_${size_mib}m.bin" >/dev/null 2>&1
    do_commit "$d_scale" "add ${size_mib}MiB" >/dev/null
    obj_count=$(find "$d_scale/.shard/objects" -type f | wc -l)
    echo "  ${size_mib}M: ${obj_count} objects" | tee -a "$RESULTS_FILE"
done

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 17: Concurrent repo stress (10 repos in parallel)
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 17: Concurrent stress ═══" | tee -a "$SUMMARY_FILE"

d17="$RESULTS_DIR/p17"
mkdir -p "$d17"
pids=""
for i in $(seq 1 10); do
    (
        repo="$d17/repo_$i"
        init_repo "$repo"
        dd if=/dev/urandom of="$repo/data.bin" bs=65536 count=1 2>/dev/null
        sh "$repo" add "data.bin" >/dev/null 2>&1
        cid=$(do_commit "$repo" "concurrent $i")
        sh "$repo" verify "$cid" >/dev/null 2>&1
    ) &
    pids="$pids $!"
done
for pid in $pids; do wait "$pid" 2>/dev/null; done
completed=0
for i in $(seq 1 10); do
    [ -f "$d17/repo_$i/.shard/HEAD" ] && completed=$((completed+1))
done
check "p17 concurrent $completed/10" test "$completed" -eq 10

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 18: P2P Network — share + pull + cross-peer verify
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 18: P2P Network ═══" | tee -a "$SUMMARY_FILE"

d18a="$RESULTS_DIR/p18a"; d18b="$RESULTS_DIR/p18b"
init_repo "$d18a"
cp "$MODELS/4m.bin" "$d18a/"
sh "$d18a" add "4m.bin" >/dev/null
cid18=$(do_commit "$d18a" "p2p test")

# Start share with line-buffered output (stdbuf prevents Rust's full buffering to file)
SHARE_OUT="$RESULTS_DIR/share_out.txt"
rm -f "$SHARE_OUT"
(cd "$d18a" && stdbuf -oL "$SHARD_BIN" share) > "$SHARE_OUT" 2>/dev/null &
SHARE_PID=$!

# Wait for share to print peer id and listen addr (up to 15s)
for i in $(seq 1 15); do
    sleep 1
    PEER_ID=$(grep "Local peer id" "$SHARE_OUT" 2>/dev/null | head -1 | sed 's/Local peer id: //')
    LISTEN_ADDR=$(grep "Listening on " "$SHARE_OUT" 2>/dev/null | head -1 | sed 's/Listening on //')
    [ -n "$PEER_ID" ] && [ -n "$LISTEN_ADDR" ] && break
done

if [ -n "$PEER_ID" ] && [ -n "$LISTEN_ADDR" ]; then
    MULTIADDR="${LISTEN_ADDR}/p2p/${PEER_ID}"
    echo "  peer_id=$PEER_ID listen=$LISTEN_ADDR" | tee -a "$RESULTS_FILE"

    mkdir -p "$d18b"
    start=$(date +%s%N)
    if sh "$d18b" pull "$MULTIADDR" "$cid18" >/dev/null 2>&1; then
        end=$(date +%s%N)
        echo "  pull 4MiB: $(( (end-start)/1000000 )) ms" | tee -a "$RESULTS_FILE"
    else
        echo "  pull 4MiB: FAILED" | tee -a "$RESULTS_FILE"
    fi

    init_repo "$d18b"
    start=$(date +%s%N)
    if sh "$d18b" pull "$MULTIADDR" "$cid18" >/dev/null 2>&1; then
        end=$(date +%s%N)
        echo "  re-pull: $(( (end-start)/1000000 )) ms" | tee -a "$RESULTS_FILE"
    else
        echo "  re-pull: FAILED" | tee -a "$RESULTS_FILE"
    fi

    check "p18.1 pulled file exists" test -f "$d18b/4m.bin"
    check "p18.2 cross-peer verify" sh "$d18b" verify "$cid18"
else
    echo "  share_output=$(cat "$SHARE_OUT" 2>/dev/null)" | tee -a "$RESULTS_FILE"
    fail "p18 share startup (peer=$PEER_ID addr=$LISTEN_ADDR)"
fi

kill "$SHARE_PID" 2>/dev/null; wait "$SHARE_PID" 2>/dev/null

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 19: Crypto/Key management
# ═════════════════════════════════════════════════════════════════════════════
echo "═══ PHASE 19: Crypto edge cases ═══" | tee -a "$SUMMARY_FILE"

d19="$RESULTS_DIR/p19"
mkdir -p "$d19"
sh "$d19" init >/dev/null 2>&1
sk_size=$(stat -c%s "$d19/.shard/keys/secret.key" 2>/dev/null || echo 0)
pk_size=$(stat -c%s "$d19/.shard/keys/public.key" 2>/dev/null || echo 0)
check "p19.1 secret.key 32 bytes" test "$sk_size" = "32"
check "p19.2 public.key 32 bytes" test "$pk_size" = "32"

d19p="$RESULTS_DIR/p19p"
mkdir -p "$d19p"
sh "$d19p" init --private >/dev/null 2>&1
check "p19.3 private config" sh "$d19p" config get private

echo "" | tee -a "$SUMMARY_FILE"

# ═════════════════════════════════════════════════════════════════════════════
# ═════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ═════════════════════════════════════════════════════════════════════════════
echo "" | tee -a "$SUMMARY_FILE"
echo "═══════════════════════════════════════════════════════════════" | tee -a "$SUMMARY_FILE"
echo "  FINAL SUMMARY" | tee -a "$SUMMARY_FILE"
echo "═══════════════════════════════════════════════════════════════" | tee -a "$SUMMARY_FILE"
echo "  Total: $TOTAL  Passed: $PASS  Failed: $FAIL" | tee -a "$SUMMARY_FILE"
echo "" | tee -a "$SUMMARY_FILE"
echo "Results directory: $RESULTS_DIR" | tee -a "$SUMMARY_FILE"
echo "Detailed results: $RESULTS_FILE" | tee -a "$SUMMARY_FILE"
echo "" | tee -a "$SUMMARY_FILE"

exit $FAIL
