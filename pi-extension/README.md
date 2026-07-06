# pi-claude-switcher

A [pi](https://github.com/badlogic/pi-mono) extension for
[claude-switcher](../). It surfaces the active Claude account inside pi and lets
you switch accounts **without leaving your session**.

Three features:

1. **Footer status** — the active account, a mini usage bar, percent, and
   reset countdown, shown on the right of pi's path row (so the footer stays two
   rows tall):

   ```
   ~/repos/claude-switcher (main)          paul-nhost  █░░░░░░░ 13% resets in 3h 30m
   ↑12k ↓3k $0.412 (sub) 8.1%/200k         claude-opus-4-8 • high
   ```

   The account segment degrades gracefully as the terminal narrows: it drops the
   "resets in" wording, then the countdown, then the bar, keeping at least the
   account name.

2. **`/claude-switcher [account]`** — change the active Claude account in-session. It
   flips the claude-switcher symlink, re-points the running pi process at the new
   profile (`CLAUDE_CONFIG_DIR`), and reloads so the conversation rebuilds under
   the new account. Your terminal and history are preserved; your next message
   runs on the new account.

   - `/claude-switcher` with no argument opens a picker (the active account is marked).
   - `/claude-switcher takeyoung` switches directly; account names tab-complete.

3. **Account pinning** — on session start the extension captures the active
   account and exports `CLAUDE_SWITCHER_PIN` for the pi process, so every
   `claude-switcher-exec` the bridge spawns stays on that one account for the
   whole conversation. Without it, an account switch elsewhere (or a machine
   whose symlink is repointed underneath a long session) would scatter the
   bridge's underlying Claude Code sessions across profile dirs and break
   resume. An explicit `/claude-switcher` re-pins to the new account; the pin is
   captured once per session, never per turn.

## Requirements

- [`claude-switcher`](../) installed and on `PATH` (or at `~/.local/bin`,
  `/usr/local/bin`, or `/opt/homebrew/bin`).
- pi driving Claude via
  [`pi-claude-bridge`](https://www.npmjs.com/package/@vanillagreen/pi-claude-bridge)
  with `pathToClaudeCodeExecutable` pointed at `claude-switcher-exec` (see the
  root README). The `/claude-switcher` continuity relies on that setup.

## Install

Once published to npm:

```sh
pi install npm:pi-claude-switcher
```

Or from a local checkout of this repo:

```sh
pi install ./pi-extension
```

### Local development

Symlink the source into pi's extension directory so edits take effect on
`/reload`:

```sh
ln -s "$PWD/pi-extension" ~/.pi/agent/extensions/claude-switcher
```

## Develop

```sh
npm install
npm run check   # type-check against pi's types
npm test        # footer layout / degradation test (no pi required)
```

## Notes & caveats

- The footer usage data is the only network-touching path; it is best-effort and
  polls `claude-switcher usage --json` every 60s. If usage is unavailable, the
  footer falls back to showing the path across the full width.
- `setFooter` replaces pi's built-in footer wholesale, so the token/model row is
  reproduced here. If pi changes its stock footer, this may need to track it.
- Switching only takes effect between turns. `/claude-switcher` cannot hot-swap a Claude
  subprocess that is already mid-turn.
