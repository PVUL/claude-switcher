#!/bin/sh
# session-sync.sh — sync pi's session transcripts across machines via git.
#
# pi keeps the authoritative conversation transcripts under
# ~/.pi/agent/sessions/<cwd>/<ts>_<uuid>.jsonl. This makes that directory a git
# repo and pushes/pulls it to a private remote so every machine converges. The
# bridge rebuilds its underlying Claude Code session from these transcripts, so
# syncing this directory alone is enough for cross-machine resume.
#
# Subcommands:
#   init <git-remote>   init the repo in the sessions dir, add remote, first sync
#   pull                fetch + integrate remote changes (bounded; run at session start)
#   push [--detach]     commit local changes + push (debounced; --detach returns at once)
#   sync                pull then push (what the timer runs)
#   status              show branch / ahead-behind / last push / remote
#   install-timer       install a launchd (macOS) / systemd (Linux) periodic `sync`
#   uninstall-timer     remove that timer
#
# Everything is best-effort: network/git failures never abort a caller. Only
# the sessions directory is versioned, so credentials outside it never sync.
#
# Env overrides:
#   PI_SESSIONS_DIR   sessions dir           (default ~/.pi/agent/sessions)
#   SYNC_BRANCH       branch name            (default main)
#   SYNC_DEBOUNCE     seconds between pushes  (default 20)
#   SYNC_INTERVAL     timer period, seconds   (default 120)

set -u

SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
DIR="${PI_SESSIONS_DIR:-$HOME/.pi/agent/sessions}"
BRANCH="${SYNC_BRANCH:-main}"
DEBOUNCE="${SYNC_DEBOUNCE:-20}"
INTERVAL="${SYNC_INTERVAL:-120}"

# State (lock, debounce stamp, log) lives OUTSIDE the repo so it never commits.
STATE_DIR="$(dirname "$DIR")/.session-sync-state"
LOCK_DIR="$STATE_DIR/lock"
STAMP="$STATE_DIR/last-push"
LOG="$STATE_DIR/sync.log"

git_dir() { git -C "$DIR" "$@"; }

is_repo() { [ -d "$DIR/.git" ]; }

# Silently no-op for lifecycle callers when the repo isn't set up yet.
require_repo_quiet() { is_repo || exit 0; }

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >>"$LOG" 2>/dev/null || true; }

# --- debounce + locking (portable: mkdir is atomic on mac and linux) --------

debounced() {
	[ -f "$STAMP" ] || return 1
	last=$(cat "$STAMP" 2>/dev/null || echo 0)
	now=$(date +%s)
	[ $((now - last)) -lt "$DEBOUNCE" ]
}

with_lock() {
	mkdir -p "$STATE_DIR" 2>/dev/null || true
	# Non-blocking: if a push is already running, just skip this one.
	if ! mkdir "$LOCK_DIR" 2>/dev/null; then
		return 0
	fi
	trap 'rmdir "$LOCK_DIR" 2>/dev/null || true' EXIT INT TERM
	"$@"
	rmdir "$LOCK_DIR" 2>/dev/null || true
	trap - EXIT INT TERM
}

run_detached() {
	mkdir -p "$STATE_DIR" 2>/dev/null || true
	if command -v setsid >/dev/null 2>&1; then
		setsid sh "$SELF" push >>"$LOG" 2>&1 </dev/null &
	else
		nohup sh "$SELF" push >>"$LOG" 2>&1 </dev/null &
	fi
}

# --- git operations ---------------------------------------------------------

# True if the repo is mid-merge/rebase or has unmerged (conflicted) paths. In
# these states `git add -A` would stage conflict markers and a blind commit would
# bake them into history — which once broke .gitignore (a non-.jsonl file, so the
# `merge=union` driver didn't apply). Never commit while this is true.
in_conflict_state() {
	[ -e "$DIR/.git/MERGE_HEAD" ] && return 0
	[ -d "$DIR/.git/rebase-merge" ] && return 0
	[ -d "$DIR/.git/rebase-apply" ] && return 0
	[ -n "$(git_dir ls-files -u 2>/dev/null)" ] && return 0
	return 1
}

# Roll back a half-finished integration so the working tree stays clean and any
# local (unpushed) commits are preserved; the next sync retries. Covers all three
# shapes: an interrupted rebase, an interrupted merge, and — the realistic one — a
# conflicted `--autostash` pop, which leaves an unmerged index with no MERGE_HEAD;
# there we restore just the conflicted paths from HEAD (session .jsonl are never
# the conflict, so nothing is lost).
abort_integration() {
	git_dir rebase --abort >>"$LOG" 2>&1 || true
	git_dir merge --abort >>"$LOG" 2>&1 || true
	# Restore the conflicted paths from HEAD (remote wins for the rare config-file
	# clash — session .jsonl never conflict thanks to merge=union).
	for f in $(git_dir ls-files -u 2>/dev/null | awk '{print $4}' | sort -u); do
		git_dir checkout HEAD -- "$f" >>"$LOG" 2>&1 || true
	done
	# Drop the leftover autostash we just undid; its non-conflicting parts (any
	# session writes) are already in the working tree and get committed next.
	if git_dir stash list 2>/dev/null | head -1 | grep -q autostash; then
		git_dir stash drop >>"$LOG" 2>&1 || true
	fi
}

do_pull() {
	# NB: `git pull --autostash` exits 0 even when the autostash pop conflicts
	# ("your changes are safe in the stash"), so decide by inspecting the repo,
	# not the return code.
	git_dir pull --rebase --autostash origin "$BRANCH" >>"$LOG" 2>&1 || log "pull returned nonzero"
	if in_conflict_state; then
		log "pull left a conflict (a non-.jsonl file, e.g. .gitignore) — taking remote, tree left clean; local commits preserved"
		abort_integration
	fi
}

do_push() {
	debounced && return 0 # authoritative re-check inside the lock
	if in_conflict_state; then
		log "skip push: repo in conflict/merge state — resolve manually in $DIR"
		return 0
	fi
	git_dir add -A >>"$LOG" 2>&1 || true
	if ! git_dir diff --cached --quiet 2>/dev/null; then
		host=$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo host)
		git_dir commit -q -m "sync: $host $(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$LOG" 2>&1 || true
	fi
	git_dir pull --rebase --autostash origin "$BRANCH" >>"$LOG" 2>&1 || log "pull-before-push failed"
	git_dir push origin "HEAD:$BRANCH" >>"$LOG" 2>&1 || log "push failed"
	date +%s >"$STAMP" 2>/dev/null || true
}

# --- subcommands ------------------------------------------------------------

cmd_init() {
	remote="${1:-}"
	[ -n "$remote" ] || { echo "usage: $(basename "$0") init <git-remote>" >&2; exit 2; }
	mkdir -p "$DIR" "$STATE_DIR"
	if ! is_repo; then
		git_dir init -q
		git_dir symbolic-ref HEAD "refs/heads/$BRANCH" 2>/dev/null || true
	fi
	# Only if absent — never clobber an existing policy (a re-init writing a
	# different .gitignore is what diverged mbp vs box and broke the repo).
	[ -f "$DIR/.gitattributes" ] || printf '*.jsonl merge=union\n' >"$DIR/.gitattributes"
	[ -f "$DIR/.gitignore" ] || printf '.session-sync-state/\n' >"$DIR/.gitignore"
	if git_dir remote get-url origin >/dev/null 2>&1; then
		git_dir remote set-url origin "$remote"
	else
		git_dir remote add origin "$remote"
	fi
	git_dir add -A
	git_dir diff --cached --quiet 2>/dev/null || git_dir commit -q -m "init: $(hostname -s 2>/dev/null || echo host)"
	# Merge any existing remote history into our local sessions, then push. Merge
	# FETCH_HEAD (always set by the fetch) rather than the remote-tracking ref,
	# which a single-branch fetch doesn't reliably create.
	if git_dir fetch origin "$BRANCH" >>"$LOG" 2>&1; then
		# Distinct per-session files combine cleanly; any same-file overlap is
		# union-merged via the .gitattributes driver committed above.
		git_dir merge --allow-unrelated-histories -m "merge remote" FETCH_HEAD >>"$LOG" 2>&1 || \
			echo "note: could not auto-merge remote/$BRANCH; resolve in $DIR" >&2
	fi
	git_dir push -u origin "HEAD:$BRANCH" >>"$LOG" 2>&1 || echo "note: initial push failed (check the remote)" >&2
	echo "initialized $DIR -> $remote (branch $BRANCH)"
}

cmd_pull() {
	require_repo_quiet
	with_lock do_pull
}

cmd_push() {
	require_repo_quiet
	if [ "${1:-}" = "--detach" ]; then
		debounced && exit 0 # cheap pre-check so we don't spawn a no-op child
		run_detached
		exit 0
	fi
	with_lock do_push
}

cmd_sync() {
	is_repo || { echo "not initialized; run: $(basename "$0") init <git-remote>" >&2; exit 1; }
	with_lock do_pull_then_push
}
do_pull_then_push() { do_pull; do_push_forced; }
# sync always pushes (timer cadence already rate-limits it); skip the debounce.
do_push_forced() {
	if in_conflict_state; then
		log "skip push: repo in conflict/merge state — resolve manually in $DIR"
		return 0
	fi
	git_dir add -A >>"$LOG" 2>&1 || true
	if ! git_dir diff --cached --quiet 2>/dev/null; then
		host=$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo host)
		git_dir commit -q -m "sync: $host $(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$LOG" 2>&1 || true
	fi
	git_dir push origin "HEAD:$BRANCH" >>"$LOG" 2>&1 || log "push failed"
	date +%s >"$STAMP" 2>/dev/null || true
}

cmd_status() {
	is_repo || { echo "not initialized ($DIR)"; exit 1; }
	echo "dir:    $DIR"
	echo "remote: $(git_dir remote get-url origin 2>/dev/null || echo '(none)')"
	echo "branch: $(git_dir rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
	git_dir fetch origin "$BRANCH" >/dev/null 2>&1 || true
	ahead=$(git_dir rev-list --count "origin/$BRANCH..HEAD" 2>/dev/null || echo '?')
	behind=$(git_dir rev-list --count "HEAD..origin/$BRANCH" 2>/dev/null || echo '?')
	echo "ahead:  $ahead   behind: $behind"
	if [ -f "$STAMP" ]; then
		echo "last push: $(date -r "$(cat "$STAMP")" 2>/dev/null || echo '?')"
	else
		echo "last push: never"
	fi
}

cmd_install_timer() {
	is_repo || { echo "run 'init' first" >&2; exit 1; }
	case "$(uname -s)" in
	Darwin)
		plist="$HOME/Library/LaunchAgents/dev.claude-switcher.pi-session-sync.plist"
		mkdir -p "$(dirname "$plist")"
		cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>dev.claude-switcher.pi-session-sync</string>
  <key>ProgramArguments</key>
  <array><string>/bin/sh</string><string>$SELF</string><string>sync</string></array>
  <key>StartInterval</key><integer>$INTERVAL</integer>
  <key>RunAtLoad</key><true/>
</dict></plist>
EOF
		launchctl unload "$plist" 2>/dev/null || true
		launchctl load "$plist" 2>/dev/null || true
		echo "launchd agent installed ($INTERVAL s): $plist"
		;;
	Linux)
		u="$HOME/.config/systemd/user"
		mkdir -p "$u"
		cat >"$u/pi-session-sync.service" <<EOF
[Unit]
Description=Sync pi session history
[Service]
Type=oneshot
ExecStart=/bin/sh $SELF sync
EOF
		cat >"$u/pi-session-sync.timer" <<EOF
[Unit]
Description=Periodic pi session history sync
[Timer]
OnBootSec=1min
OnUnitActiveSec=${INTERVAL}s
[Install]
WantedBy=timers.target
EOF
		systemctl --user daemon-reload 2>/dev/null || true
		systemctl --user enable --now pi-session-sync.timer 2>/dev/null || \
			echo "enable failed; try: systemctl --user enable --now pi-session-sync.timer"
		echo "systemd user timer installed (${INTERVAL}s): $u/pi-session-sync.timer"
		;;
	*)
		echo "unsupported OS for install-timer; run '$SELF sync' from cron" >&2
		exit 1
		;;
	esac
}

cmd_uninstall_timer() {
	case "$(uname -s)" in
	Darwin)
		plist="$HOME/Library/LaunchAgents/dev.claude-switcher.pi-session-sync.plist"
		launchctl unload "$plist" 2>/dev/null || true
		rm -f "$plist" && echo "removed launchd agent"
		;;
	Linux)
		systemctl --user disable --now pi-session-sync.timer 2>/dev/null || true
		rm -f "$HOME/.config/systemd/user/pi-session-sync.timer" \
			"$HOME/.config/systemd/user/pi-session-sync.service"
		systemctl --user daemon-reload 2>/dev/null || true
		echo "removed systemd timer"
		;;
	esac
}

# Keep the log from growing without bound.
[ -f "$LOG" ] && [ "$(wc -c <"$LOG" 2>/dev/null || echo 0)" -gt 1000000 ] && : >"$LOG"

case "${1:-}" in
init) shift; cmd_init "$@" ;;
pull) cmd_pull ;;
push) shift; cmd_push "$@" ;;
sync) cmd_sync ;;
status) cmd_status ;;
install-timer) cmd_install_timer ;;
uninstall-timer) cmd_uninstall_timer ;;
*)
	echo "usage: $(basename "$0") {init <remote>|pull|push [--detach]|sync|status|install-timer|uninstall-timer}" >&2
	exit 2
	;;
esac
