# claudesub

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
variable. `claudesub` keeps one directory per account and flips a single symlink
to choose the active one:

```
~/.claude-work        ← a complete Claude config dir
~/.claude-personal    ← another one
~/.claude-client      ← another one
~/.claude-active  ->  ~/.claude-work   (the symlink; the source of truth)
```

Point anything at `~/.claude-active` and it uses whichever account is active.
**No files are ever copied.** Switching only re-points the symlink, atomically.

## Install

Requires [Rust](https://rustup.rs). From the repo:

```sh
./install.sh                    # installs to ~/.local/bin
PREFIX=/usr/local ./install.sh  # or a system prefix
```

This installs two things:

- `claudesub` — the manager (CLI + TUI).
- `claude-active` — a tiny wrapper that runs Claude under the active profile.

## Wire up your tools

Pick whichever fits; they all read the same symlink:

```sh
# Simplest: alias the CLI in your shell profile
alias claude='claude-active'

# Or export the variable directly (what `claude-active` does under the hood)
export CLAUDE_CONFIG_DIR="$HOME/.claude-active"
```

- **Pi wrapper:** launch `claude-active` instead of `claude`.
- **pi-claude-bridge:** set `pathToClaudeCodeExecutable` to the `claude-active`
  path.

`claudesub env` prints the export line for you.

## Usage

```sh
claudesub                      # open the interactive TUI
claudesub adopt --scan         # auto-import existing ~/.claude[-*] dirs as profiles
claudesub adopt                # adopt the default ~/.claude (name "default")
claudesub adopt work --path ~/.claude-work   # adopt one directory in place
claudesub add work             # create ~/.claude-work (first one becomes active)
claudesub add client --path ~/work/.claude-acme   # custom directory
claudesub switch work          # re-point the symlink atomically
claudesub current              # print the active profile (+ email)
claudesub list                 # detailed listing
claudesub list --json          # machine-readable
claudesub rename work client   # renames + moves the dir if at the default path
claudesub remove client        # unmanage (directory kept on disk)
claudesub remove client --purge  # unmanage AND delete the directory
claudesub env                  # print shell setup
```

### Importing your current setup

Already using Claude Code? Adopt your existing config directories as profiles
instead of starting over. Nothing is copied — `adopt` registers the directory
in place and the symlink starts pointing at it.

```sh
claudesub adopt --scan   # finds ~/.claude and every ~/.claude-* directory
```

The default `~/.claude` is a special case: its login/onboarding state lives in
`~/.claude.json` (beside the directory, not inside it). To carry that into the
profile so it works under the wrapper, add `--migrate-state` — this *copies*
`~/.claude.json` into the profile dir and leaves the original untouched, so a
bare `claude` keeps working too:

```sh
claudesub adopt --migrate-state    # adopt ~/.claude and import its login state
```

Other directories (like `~/.claude-work`) already keep `.claude.json` inside
them, so no migration is needed.

### First-time sign-in for a profile

A freshly added profile is an empty config dir, so it shows as *not signed in*:

```sh
claudesub add work
claudesub switch work
claude-active        # or `claude` if aliased — sign in normally
```

Once signed in, `claudesub` reads the account email from the profile's
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

## How it works

- **Symlink is authoritative.** The active profile is always determined by
  reading `~/.claude-active`. On startup `claudesub` reconciles its metadata
  cache with the symlink, so external changes are respected.
- **Atomic switching.** A new symlink is created at a temp path and `rename(2)`d
  over the old one — an all-or-nothing swap, never a half state.
- **Metadata is UI-only.** `~/.config/claudesub/profiles.json` stores display
  order, last-used times and cached emails. It never decides what's active.
- **Local detection only.** Email and "authenticated" status are read from the
  files Claude already writes (`.claude.json`, `.credentials.json`). No API
  calls, no quota checks, no network.

Metadata schema:

```json
{
  "active": "work",
  "profiles": [
    {
      "name": "work",
      "path": "~/.claude-work",
      "lastUsed": "2026-07-04T09:21:00Z",
      "email": "paul@nhost.io"
    }
  ]
}
```

Override locations (handy for testing or non-standard setups):

- `CLAUDESUB_HOME` — where profile dirs and the symlink live (default `$HOME`).
- `CLAUDESUB_CONFIG_DIR` — where `profiles.json` lives (default
  `~/.config/claudesub`).

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
| `tui/`              | Ratatui UI (`app` state, `ui` render, event loop) |

## License

MIT — see [LICENSE](LICENSE).
