# claude-switcher

Switch between multiple isolated **Claude Code** accounts with a single atomic
symlink — and make every tool (the `claude` CLI, Pi, `pi-claude-bridge`, any
future wrapper) follow the active account automatically.

```
┌──────────────────────────────────────────────┐
│  Claude Accounts   active: work                │
├──────────────────────────────────────────────┤
│ Profiles                                       │
│ > ✓ work (paul@nhost.io)                       │
│      last used: 2 min ago                      │
│   personal (paul@personal.dev)                 │
│      last used: yesterday                      │
│   client (acme@client.com)                     │
│      last used: never   [not signed in]        │
├──────────────────────────────────────────────┤
│ ENTER switch   A add   R rename   D delete  Q  │
└──────────────────────────────────────────────┘
```

## Why

Claude Code isolates *everything* — credentials, settings, history, plugins,
skills — inside the directory named by the `CLAUDE_CONFIG_DIR` environment
variable. `claude-switcher` keeps one directory per account and flips a single symlink
to choose the active one:

```
~/.claude-work        ← a complete Claude config dir
~/.claude-personal    ← another one
~/.claude-client      ← another one
~/.claude-switcher  ->  ~/.claude-work   (the symlink; the source of truth)
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
# Recommended: export the variable in your shell profile so the plain `claude`
# command follows the active profile.
export CLAUDE_CONFIG_DIR="$HOME/.claude-switcher"
```

- **pi-claude-bridge / tools that want an executable path:** point
  `pathToClaudeCodeExecutable` at `claude-switcher-exec`.
- **Pi wrapper:** launch `claude-switcher-exec` instead of `claude` (or rely on
  the exported variable above).

`claude-switcher env` prints the export line for you.

## Usage

```sh
claude-switcher                      # open the interactive TUI
claude-switcher adopt --scan         # auto-import existing ~/.claude[-*] dirs as profiles
claude-switcher adopt                # adopt the default ~/.claude (name "default")
claude-switcher adopt work --path ~/.claude-work   # adopt one directory in place
claude-switcher add work             # create ~/.claude-work (first one becomes active)
claude-switcher add client --path ~/work/.claude-acme   # custom directory
claude-switcher switch work          # re-point the symlink atomically
claude-switcher current              # print the active profile (+ email)
claude-switcher usage                # per-account usage limits (5-hour / 7-day)
claude-switcher list                 # detailed listing
claude-switcher list --json          # machine-readable
claude-switcher rename work client   # renames + moves the dir if at the default path
claude-switcher remove client        # unmanage (directory kept on disk)
claude-switcher remove client --purge  # unmanage AND delete the directory
claude-switcher env                  # print shell setup
```

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

A freshly added profile is an empty config dir, so it shows as *not signed in*:

```sh
claude-switcher add work
claude-switcher switch work
claude               # follows the active profile via CLAUDE_CONFIG_DIR
```

Once signed in, `claude-switcher` reads the account email from the profile's
`.claude.json` and shows it in parentheses.

### TUI keys

| Key            | Action                          |
| -------------- | ------------------------------- |
| `↑`/`k`, `↓`/`j` | Move selection                |
| `Enter`        | Switch to selected profile      |
| `A`            | Add a profile                   |
| `R`            | Rename the selected profile     |
| `D`            | Delete (unmanage) the profile   |
| `Q` / `Esc`    | Quit                            |

No mouse required.

The list is ordered active-first, then by most-recent usage. That order is
fixed when the TUI opens and stays put while you navigate — switching moves the
✓ but never reshuffles the rows out from under you. (`claude-switcher list`
re-sorts on each run.)

## Usage limits

`claude-switcher usage` shows how much of each account's rate limits you've
consumed, and the TUI displays the same per row (fetched in the background, so
the UI opens instantly):

```
* takeyoung (takeyoung@gmail.com)
      5-hour:   0%   (resets in 3h 36m)
      7-day :   0%   (resets in 4d)
  paul-nhost (paul@nhost.io)
      5-hour:  20%   (resets in 3h 36m)
      7-day :   4%   (resets in 17h 46m)
```

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
      "email": "paul@nhost.io"
    }
  ]
}
```

Override locations (handy for testing or non-standard setups):

- `CLAUDE_SWITCHER_HOME` — where profile dirs and the symlink live (default `$HOME`).
- `CLAUDE_SWITCHER_CONFIG_DIR` — where `profiles.json` lives (default
  `~/.config/claude-switcher`).

## Development

```sh
cargo test        # unit + end-to-end tests
cargo run         # launch the TUI against your real $HOME
cargo build --release
```

Module layout:

| File                | Responsibility                                   |
| ------------------- | ------------------------------------------------ |
| `paths.rs`          | Location resolution + tilde expand/contract      |
| `symlink.rs`        | Atomic symlink + file operations                 |
| `metadata.rs`       | `profiles.json` load/save                        |
| `detect.rs`         | Local email / auth detection                     |
| `profile.rs`        | Runtime profile type + name validation           |
| `manager.rs`        | Orchestration; keeps symlink ⇄ metadata in sync  |
| `cli.rs` / `commands.rs` | Non-interactive command surface             |
| `usage.rs`          | Opt-in usage-limit lookup (token + endpoint)     |
| `tui/`              | Ratatui UI (`app` state, `ui` render, event loop) |

## License

MIT — see [LICENSE](LICENSE).
