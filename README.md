# claude-switcher

Switch between multiple isolated **Claude Code** accounts with a single atomic
symlink — and make every tool (the `claude` CLI, Pi, `pi-claude-bridge`, any
future wrapper) follow the active account automatically.

```sh
┌─────────────────────────────────────────────────────────────────────────┐
│  Claude Switcher        updated 3:49pm       auto-refresh: on           │
├─ Accounts ──────────────────────────────────────────────────────────────┤
│› ✓ work  (you@work.com · Team)                                          │
│      5h [██████░░░░░░░░░░]  22%   resets in 3h 30m  (3:50pm)            │
│      7d [█░░░░░░░░░░░░░░░]   5%   resets in 17h 40m (Sun 6:00am)        │
│      last used: 2 min ago                                               │
│  personal  (you@personal.dev · Pro)                                     │
│      5h [░░░░░░░░░░░░░░░░]   0%   resets in 4h 12m  (7:01pm)            │
│      7d [██░░░░░░░░░░░░░░]  11%   resets in 3d 2h  (Tue 9:00am)         │
│      last used: yesterday                                               │
│  client                                                                 │
│      usage unavailable                                                  │
│      last used: never   [not signed in]                                 │
├─────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move · enter switch · a add · e edit · d delete · r refresh · q quit │
└─────────────────────────────────────────────────────────────────────────┘
```

## Why

Claude Code isolates *everything* — credentials, settings, history, plugins,
skills — inside the directory named by the `CLAUDE_CONFIG_DIR` environment
variable. `claude-switcher` keeps one directory per account and flips a single symlink
to choose the active one:

```sh
~/.claude-work                           # ← a complete Claude config dir
~/.claude-personal                       # ← another one
~/.claude-client                         # ← another one
~/.claude-switcher  ->  ~/.claude-work   # (the symlink; the source of truth)
```

Point anything at `~/.claude-switcher` and it uses whichever account is active.
**No files are ever copied.** Switching only re-points the symlink, atomically.

## Install

Requires [Rust](https://rustup.rs). From the repo:

```sh
./install.sh                    # installs to ~/.local/bin
PREFIX=/usr/local ./install.sh  # or a system prefix
```

This installs two things:

- `claude-switcher` — the manager (CLI + TUI).
- `claude-switcher-exec` — a tiny wrapper that runs Claude under the active
  profile, for tools that take an executable path instead of an environment.

## Wire up your tools

Pick whichever fits; they all read the same symlink:

```sh
# Recommended: live shell integration. Switching accounts then takes effect in
# the CURRENT shell — no new terminal needed (see "Switching without a new
# terminal" below).
eval "$(claude-switcher shellenv)"

# Simpler alternative: just export the variable. The plain `claude` command
# follows the active profile, but an in-shell switch only affects shells started
# afterwards.
export CLAUDE_CONFIG_DIR="$HOME/.claude-switcher"
```

- **pi-claude-bridge / tools that want an executable path:** point
  `pathToClaudeCodeExecutable` at `claude-switcher-exec`.
- **Pi wrapper:** launch `claude-switcher-exec` instead of `claude` (or rely on
  the exported variable above).

`claude-switcher env` prints the export line; `claude-switcher shellenv` prints
the live integration.

## Switching without a new terminal

On macOS, Claude Code keys its Keychain OAuth token by a hash of the *literal*
`CLAUDE_CONFIG_DIR` string. So the variable must hold the **resolved** profile
path (e.g. `~/.claude-work`), not the symlink path — otherwise every profile
would share one token slot. That resolved path is captured once at shell start,
which is why a plain `export` leaves already-open shells stuck on the old
account after you switch.

`claude-switcher shellenv` fixes this. It defines a wrapper function that runs
the real command, then re-resolves and re-exports `CLAUDE_CONFIG_DIR` in the
current shell:

```sh
eval "$(claude-switcher shellenv)"   # in ~/.zshenv, ~/.zshrc, or ~/.bashrc

claude-switcher switch work          # symlink flips AND $CLAUDE_CONFIG_DIR updates here
claude                               # already uses the 'work' account — same terminal
```

A switch made inside the TUI propagates too (the wrapper re-syncs after the TUI
exits). No files are copied and each profile keeps its own Keychain token slot.

## Pinning an account for a long-lived session

`claude-switcher-exec` normally re-reads the symlink on **every** launch, so it
always follows the currently-active account. That is exactly wrong for a
long-running conversation whose turns must all stay on one account. Tools like
the pi claude-bridge spawn the wrapper once per turn; if the active account
changes mid-conversation (a switch elsewhere, or a machine whose symlink is
repointed underneath it), successive turns land in different profile dirs and
the underlying Claude Code sessions scatter — breaking resume.

Set `CLAUDE_SWITCHER_PIN` to a **resolved** profile directory to pin an
invocation to that account regardless of the symlink:

```sh
CLAUDE_SWITCHER_PIN="$HOME/.claude-work" claude-switcher-exec   # this run uses 'work', period
```

The pin must name an existing directory (and be the resolved profile path, not
the `~/.claude-switcher` symlink — same Keychain-hashing reason as above); if it
doesn't, the wrapper warns and falls back to following the active symlink. A
harness can capture the active profile once at session start and export the pin
for every child launch, so the whole conversation stays on one account.

The read-only reporting commands honor the pin too: with `CLAUDE_SWITCHER_PIN`
set to a managed profile directory, `claude-switcher current`, `list`, and
`usage` report **that** account as active rather than the global symlink target.
So a pinned session (and anything it asks, like an agent introspecting "which
account am I on") sees the account it actually runs on, even after the symlink
is flipped elsewhere. Switching, the TUI, and the symlink itself are unaffected
— the pin only shifts what a pinned session *reports*, never the global state.

The read-only reporting commands honor the pin too: with `CLAUDE_SWITCHER_PIN`
set to a managed profile directory, `claude-switcher current`, `list`, and
`usage` report **that** account as active rather than the global symlink target.
So a pinned session (and anything it asks, like an agent introspecting "which
account am I on") sees the account it actually runs on, even after the symlink
is flipped elsewhere. Switching, the TUI, and the symlink itself are unaffected
— the pin only shifts what a pinned session *reports*, never the global state.

## Usage

```sh
claude-switcher                                         # open the interactive TUI
claude-switcher adopt --scan                            # auto-import existing ~/.claude[-*] dirs as profiles
claude-switcher adopt                                   # adopt the default ~/.claude (name "default")
claude-switcher adopt work --path ~/.claude-work        # adopt one directory in place
claude-switcher adopt work --activate                   # ...and make it active (single-profile mode)
claude-switcher add work                                # create ~/.claude-work (first one becomes active)
claude-switcher add client --path ~/work/.claude-acme   # custom directory
claude-switcher switch work                             # re-point the symlink atomically
claude-switcher current                                 # print the active profile (+ email)
claude-switcher usage                                   # per-account usage limits (5-hour / 7-day)
claude-switcher usage --json                            # machine-readable
claude-switcher list                                    # detailed listing
claude-switcher list --json                             # machine-readable
claude-switcher rename work client                      # renames + moves the dir if at the default path
claude-switcher remove client                           # unmanage (directory kept on disk)
claude-switcher remove client --purge                   # unmanage AND delete the directory
claude-switcher env                                     # print shell setup (static export)
claude-switcher shellenv                                # print live shell integration (in-shell switching)
claude-switcher doctor                                  # diagnose + repair setup (see below)
claude-switcher doctor -y                               # ...applying all safe fixes without prompting
```

### `doctor` — set up or fix a machine

`claude-switcher doctor` walks the whole setup and repairs what it safely can:
it adopts the Claude config directories already on the machine, activates a
profile (the only one, or one you pick), then checks that each profile is
*signed in* and that the active profile's **usage** endpoint is reachable —
reporting exactly what's wrong and the command to fix it. Safe, additive fixes
(adopting a directory, activating the sole profile) are applied automatically;
anything that needs a human (choosing among accounts, signing in through a
browser) is guided, never forced.

It also runs **automatically on launch** (before the TUI) when setup looks
incomplete, and stays silent when everything is healthy. Non-interactive callers
(pipes, the pi extension's `usage --json`) never trigger it; set
`CLAUDE_SWITCHER_NO_WIZARD=1` to opt out entirely.

Decline an adoption prompt and it **stays declined** — the directory is recorded
as ignored so the wizard won't re-offer it on the next launch (useful for a bare
`~/.claude` you never signed a real account into). You can still adopt it later
by hand with `claude-switcher adopt <name> --path <dir>`, which clears the mark.

A common headless case it names precisely: if `CLAUDE_CODE_OAUTH_TOKEN` is set
to a coding-only `claude setup-token` (as on a server), Claude Code works but
usage/sign-in display won't — that token lacks the `user:profile` scope, so
`doctor` tells you to do a real interactive `claude` login for the profile.

### Importing your current setup

Already using Claude Code? Adopt your existing config directories as profiles
instead of starting over. Nothing is copied — `adopt` registers the directory
in place and the symlink starts pointing at it.

```sh
claude-switcher adopt --scan   # finds ~/.claude and every ~/.claude-* directory
```

The default `~/.claude` is a special case: its login/onboarding state lives in
`~/.claude.json` (beside the directory, not inside it). To carry that into the
profile so it works under the wrapper, add `--migrate-state` — this *copies*
`~/.claude.json` into the profile dir and leaves the original untouched, so a
bare `claude` keeps working too:

```sh
claude-switcher adopt --migrate-state    # adopt ~/.claude and import its login state
```

Other directories (like `~/.claude-work`) already keep `.claude.json` inside
them, so no migration is needed.

### First-time sign-in for a profile

A freshly added profile is an empty config dir, so it shows as *not signed in*.
Adding one in the TUI (`A`) prompts you to sign in right away — press `Enter`
and it activates the new profile and launches `claude` so you can log in
(`Esc` skips and leaves your current account active). From the command line:

```sh
claude-switcher add work
claude-switcher switch work
claude                        # follows the active profile via CLAUDE_CONFIG_DIR
```

Once signed in, `claude-switcher` reads the account email from the profile's
`.claude.json` and shows it in parentheses.

### TUI keys

| Key              | Action                                                       |
| ---------------- | ------------------------------------------------------------ |
| `↑`/`k`, `↓`/`j` | Move selection (incl. the header auto-refresh toggle)        |
| `Enter`          | Switch to the selected profile; press it again on the now-active profile to close. Toggles auto-refresh when the header is focused |
| `R`              | Manual refresh of usage (also resets the auto-refresh timer) |
| `A`              | Add a profile (then `Enter` to sign in with Claude, `Esc` to skip) |
| `E`              | Edit (rename) the selected profile                           |
| `D`              | Delete (unmanage) the profile                                |
| `M`              | Toggle the compact (minimal) one-line-per-profile view       |
| `Q` / `Esc`      | Quit                                                         |

No mouse required.

The header shows the **last-updated time** on the left and an **auto-refresh**
toggle on the right. Move up to the toggle and press Enter to turn periodic
polling on/off (remembered across sessions). Press `R` anytime for a manual
refresh (debounced to once per minute). Parking the selector on the header also
un-highlights every profile row, so the list stays easy to read.

Press `M` for a **minimal view** that collapses each profile to a single line —
the alias, its 5-hour bar, and when that window resets — for a quick glance
without scrolling. The preference is remembered across sessions.

```sh
› ✓ work      ██████░░░░░░░░░░  22%  resets in 3h 30m  (3:50pm)
    personal  ░░░░░░░░░░░░░░░░   0%  resets in 4h 12m  (7:01pm)
    client    usage unavailable
```

Usage snapshots are cached to `profiles.json` with their fetch time. When you
reopen the TUI within the poll interval, the cached values are shown
immediately with **no API call**, and the next auto-refresh is scheduled to
land exactly at the interval mark (a 4-minute-old snapshot refreshes in 1
minute for a 5-minute poll).

Auto-refresh polls every 5 minutes by default; change `pollIntervalSecs` in
`~/.config/claude-switcher/profiles.json`:

```json
"settings": { "autoRefresh": true, "pollIntervalSecs": 300, "compactView": false }
```

The list is ordered active-first, then by most-recent usage. That order is
fixed when the TUI opens and stays put while you navigate — switching moves the
✓ but never reshuffles the rows out from under you. (`claude-switcher list`
re-sorts on each run.)

## Usage limits

`claude-switcher usage` shows how much of each account's rate limits you've
consumed, and the TUI displays the same per row (fetched in the background, so
the UI opens instantly):

```sh
* work (you@work.com · Team)
      5-hour  [████░░░░░░░░░░░░░░░░] 22%  resets in 3h 30m  (3:50pm)
      7-day   [█░░░░░░░░░░░░░░░░░░░]  5%  resets in 17h 40m (Sun 6:00am)
  personal (you@personal.dev · Pro)
      5-hour  [░░░░░░░░░░░░░░░░░░░░]  0%  resets in 3h 30m  (3:49pm)
      7-day   [░░░░░░░░░░░░░░░░░░░░]  0%  resets in 4d 5h   (Wed 5:59pm)
```

The bar is color-coded in the TUI (green / yellow / red as you approach the
limit), and each window shows both the relative countdown and the local
wall-clock time it resets. Accounts with a separate Opus allotment also get an
`opus` line (CLI) / a `· opus N%` suffix on the 7-day row (TUI) when non-zero.

This is the **only** feature that touches the network, and it's entirely
opt-in — `list`, `current`, and switching stay fully offline. It reads each
account's own OAuth token (macOS Keychain, or `<dir>/.credentials.json` on
Linux) and queries `https://api.anthropic.com/api/oauth/usage` via `curl`. If a
token is missing, expired, or you're offline, usage simply shows as
`unavailable` — nothing breaks. Requires `curl` (standard on macOS/Linux).

## How it works

- **Symlink is authoritative.** The active profile is always determined by
  reading `~/.claude-switcher`. On startup `claude-switcher` reconciles its metadata
  cache with the symlink, so external changes are respected.
- **Atomic switching.** A new symlink is created at a temp path and `rename(2)`d
  over the old one — an all-or-nothing swap, never a half state.
- **Metadata is UI-only.** `~/.config/claude-switcher/profiles.json` stores display
  order and cached emails. It never decides what's active.
- **Last-used reflects real usage.** It's derived from the mtime of the files
  Claude writes when a profile is actually used (`.claude.json`, `history.jsonl`,
  `sessions/`, `projects/`) — not from when you selected the profile. Switching
  to a profile without using it does not change its last-used time.
- **Offline by default.** Email and "authenticated" status are read from the
  files Claude already writes (`.claude.json`, `.credentials.json`) — no network.
  The single exception is the opt-in [usage limits](#usage-limits) feature.

Metadata schema:

```json
{
  "active": "work",
  "profiles": [
    {
      "name": "work",
      "path": "~/.claude-work",
      "email": "you@work.com"
    }
  ],
  "settings": { "autoRefresh": true, "pollIntervalSecs": 300, "compactView": false }
}
```

The file also carries a `usageCache` block (the last usage snapshot plus its
fetch time) so the TUI can render instantly and skip a redundant API call when
reopened within the poll interval.

Override locations (handy for testing or non-standard setups):

- `CLAUDE_SWITCHER_HOME` — where profile dirs and the symlink live (default `$HOME`).
- `CLAUDE_SWITCHER_CONFIG_DIR` — where `profiles.json` lives (default
  `~/.config/claude-switcher`).

## Pi extensions

This repo is a small monorepo: the Rust app lives at the root, plus two
[pi](https://github.com/badlogic/pi-mono) extensions.

[`pi-extension/`](pi-extension) holds **pi-claude-switcher** — surfaces the
active account + usage in pi's footer, adds a `/claude-switcher [account]`
command to change accounts without leaving your session, and pins each session
to one account (via `CLAUDE_SWITCHER_PIN`) so the bridge doesn't scatter its
underlying Claude Code sessions across profiles. See
[`pi-extension/README.md`](pi-extension/README.md).

```sh
pi install ./pi-extension      # from a checkout
# or, once published:  pi install npm:pi-claude-switcher
```

[`pi-session-sync/`](pi-session-sync) holds **pi-session-sync** — keeps pi's
session transcripts (`~/.pi/agent/sessions/`) in sync across machines through a
private git remote, driven by pi's session lifecycle (pull on start, debounced
push on each `agent_end`, flush on shutdown). See
[`pi-session-sync/README.md`](pi-session-sync/README.md).

```sh
pi install ./pi-session-sync
pi-session-sync/session-sync.sh init git@github.com:you/pi-sessions.git   # once per machine
```

## Development

```sh
cargo test             # unit + end-to-end tests
cargo run              # launch the TUI against your real $HOME
cargo build --release
```

Module layout:

| File                     | Responsibility                                    |
| ------------------------ | ------------------------------------------------- |
| `paths.rs`               | Location resolution + tilde expand/contract       |
| `symlink.rs`             | Atomic symlink + file operations                  |
| `metadata.rs`            | `profiles.json` load/save                         |
| `detect.rs`              | Local email / auth detection                      |
| `profile.rs`             | Runtime profile type + name validation            |
| `manager.rs`             | Orchestration; keeps symlink ⇄ metadata in sync   |
| `cli.rs` / `commands.rs` | Non-interactive command surface                   |
| `usage.rs`               | Opt-in usage-limit lookup (token + endpoint)      |
| `tui/`                   | Ratatui UI (`app` state, `ui` render, event loop) |

## License

MIT — see [LICENSE](LICENSE).

## Ref

Inspired by https://codelynx.dev/posts/claude-code-usage-limits-statusline
