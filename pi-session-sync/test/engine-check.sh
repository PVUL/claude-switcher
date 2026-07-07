#!/bin/sh
# End-to-end check of session-sync.sh against a local bare "remote".
# No pi and no network required.
set -eu

ENGINE="$(cd "$(dirname "$0")/.." && pwd)/session-sync.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0
check() { if [ "$2" = "$3" ]; then echo "OK  $1"; else echo "FAIL $1: expected [$3] got [$2]"; fail=1; fi; }

git init -q --bare "$tmp/remote.git"
run() { PI_SESSIONS_DIR="$1/agent/sessions" SYNC_DEBOUNCE=0 sh "$ENGINE" "$2" "$3" 2>/dev/null; }

# A: init with a local session, publishes to the remote.
mkdir -p "$tmp/A/agent/sessions/proj"
echo '{"id":"a1"}' >"$tmp/A/agent/sessions/proj/s1.jsonl"
run "$tmp/A" init "$tmp/remote.git" >/dev/null

# B: init on a second machine that already has its own local session —
# should combine A's history with B's local files, not clobber either.
mkdir -p "$tmp/B/agent/sessions/proj"
echo '{"id":"b1"}' >"$tmp/B/agent/sessions/proj/s2.jsonl"
run "$tmp/B" init "$tmp/remote.git" >/dev/null
check "init merges remote + keeps local" \
	"$(ls "$tmp/B/agent/sessions/proj" | tr '\n' ' ')" "s1.jsonl s2.jsonl "

# A adds a session; B pulls it.
echo '{"id":"a2"}' >"$tmp/A/agent/sessions/proj/s3.jsonl"
run "$tmp/A" push x >/dev/null
run "$tmp/B" pull x >/dev/null
check "pull picks up new remote sessions" \
	"$(ls "$tmp/B/agent/sessions/proj" | tr '\n' ' ')" "s1.jsonl s2.jsonl s3.jsonl "

# Same file edited on both machines -> union merge keeps both sides.
printf '{"id":"a1"}\n{"e":"A-side"}\n' >"$tmp/A/agent/sessions/proj/s1.jsonl"
run "$tmp/A" push x >/dev/null
printf '{"id":"a1"}\n{"e":"B-side"}\n' >"$tmp/B/agent/sessions/proj/s1.jsonl"
run "$tmp/B" sync x >/dev/null
grep -q '"e":"A-side"' "$tmp/B/agent/sessions/proj/s1.jsonl" && \
	grep -q '"e":"B-side"' "$tmp/B/agent/sessions/proj/s1.jsonl" && u=both || u=lost
check "union merge keeps both sides of a same-file edit" "$u" "both"

# Regression: a non-.jsonl conflict (.gitignore edited differently on both sides)
# must NEVER be committed with conflict markers, and must leave the tree usable.
# This is the failure that once baked <<<<<<< markers into .gitignore on origin.
printf 'A-ignore\n' >"$tmp/A/agent/sessions/.gitignore"
run "$tmp/A" push x >/dev/null
printf 'B-ignore\n' >"$tmp/B/agent/sessions/.gitignore"
run "$tmp/B" sync x >/dev/null 2>&1 || true
grep -q '<<<<<<<' "$tmp/B/agent/sessions/.gitignore" && m=markers || m=clean
check "non-.jsonl conflict never leaves markers in the tree" "$m" "clean"
git -C "$tmp/B/agent/sessions" grep -q '<<<<<<<' HEAD -- . 2>/dev/null && c=committed || c=clean
check "non-.jsonl conflict is never committed" "$c" "clean"
PI_SESSIONS_DIR="$tmp/B/agent/sessions" SYNC_DEBOUNCE=0 sh "$ENGINE" push >/dev/null 2>&1
check "repo left usable after a conflict (push rc 0)" "$?" "0"

# Uninitialized dirs are silent no-ops (never disrupt a pi lifecycle call).
PI_SESSIONS_DIR="$tmp/none/agent/sessions" sh "$ENGINE" pull >/dev/null 2>&1
check "pull on uninitialized repo is a no-op (rc 0)" "$?" "0"
PI_SESSIONS_DIR="$tmp/none/agent/sessions" sh "$ENGINE" push --detach >/dev/null 2>&1
check "detached push on uninitialized repo is a no-op (rc 0)" "$?" "0"

echo
[ "$fail" = 0 ] && echo "ALL OK" || { echo "FAILURES"; exit 1; }
