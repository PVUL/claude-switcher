# pi-session-sync

A [pi](https://github.com/badlogic/pi-mono) extension that keeps your pi
**session history in sync across machines** — Mac, laptop, VPS — through a
private git remote, driven by pi's own session lifecycle. No daemon.

## Why

pi stores the authoritative conversation transcripts under
`~/.pi/agent/sessions/<cwd>/<ts>_<uuid>.jsonl`. The claude-bridge *rebuilds* its
underlying Claude Code session from these transcripts when resuming, so syncing
this one directory is enough to carry a conversation to another machine — you
don't need (and shouldn't sync) the per-account `~/.claude*/projects/` dirs.

Because each session is its own file, git merges are clean in the normal
single-writer case. A `*.jsonl merge=union` attribute is the safety net for the
rare case where you touch the same session on two machines: both sides are kept,
never a conflict. The git repo is scoped **inside** `~/.pi/agent/sessions/`, so
credentials and config in the parent (`auth.json`, …) structurally can't sync.

## How it works

The engine is a standalone POSIX script (`session-sync.sh`); the extension just
calls it on pi lifecycle events:

| pi event | action |
| --- | --- |
| `session_start` (not a reload) | `pull` — bounded, so you start on fresh data |
| `agent_end` | `push --detach` — debounced, never blocks a turn |
| `session_shutdown` | `push` — final flush before exit |

`/session-sync` forces an immediate `pull + push`. Every call is best-effort: a
missing engine, an unconfigured repo, or a network hiccup is swallowed so it
never disrupts a session.

## Setup

**1. Create a private remote** (e.g. an empty private GitHub repo).

**2. Install the extension** (from a checkout of this repo):

```sh
pi install ./pi-session-sync
# or, for local dev so edits take effect on /reload:
ln -s "$PWD/pi-session-sync" ~/.pi/agent/extensions/session-sync
```

**3. Initialize the store on each machine** (once):

```sh
pi-session-sync/session-sync.sh init git@github.com:you/pi-sessions.git
```

`init` combines any history already on the remote with the sessions already on
this machine (neither is clobbered) and publishes the result.

**4. On any machine that also runs headless** (a VPS), add a periodic sync so
non-interactive use and sessions left open still converge:

```sh
pi-session-sync/session-sync.sh install-timer   # launchd on macOS, systemd --user on Linux
```

Interactive machines don't need the timer — `agent_end`/`session_shutdown`
already cover them.

## Engine commands

```sh
session-sync.sh init <git-remote>   # set up the repo, merge remote, first push
session-sync.sh pull                # integrate remote changes (bounded)
session-sync.sh push [--detach]     # commit + push (debounced; --detach returns at once)
session-sync.sh sync                # pull + push (what the timer runs)
session-sync.sh status              # branch / ahead-behind / last push / remote
session-sync.sh install-timer       # periodic `sync` via launchd/systemd
session-sync.sh uninstall-timer
```

Env overrides: `PI_SESSIONS_DIR` (default `~/.pi/agent/sessions`), `SYNC_BRANCH`
(`main`), `SYNC_DEBOUNCE` (20s between pushes), `SYNC_INTERVAL` (120s timer
period).

## Develop

```sh
npm install
npm run check   # type-check against pi's types
npm test        # end-to-end engine check against a local bare remote (no pi, no network)
```

## Caveats

- Sessions are keyed by working directory, so a machine with a different home
  path (a VPS) groups them under a different directory; a conversation is still
  found by its session id, just listed under that machine's path.
- A session actively mid-turn on one machine isn't pushed until its next
  `agent_end`, so another machine won't see the in-flight turn until then.
