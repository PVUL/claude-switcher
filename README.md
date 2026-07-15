# claude-switcher

*Monorepo.* A Rust TUI for switching between fully isolated Claude accounts,
plus the pi extensions that surface it inside your editor.

## Projects

| Path | What it is | Stack |
| ---- | ---------- | ----- |
| [`switcher/`](switcher/) | The **claude-switcher** TUI + `claude-switcher-exec` wrapper — manage and switch isolated Claude config profiles. | Rust |
| [`extensions/pi-claude-switcher/`](extensions/pi-claude-switcher/) | pi extension: active-account footer + `/claude-switcher` in-session account switching. | TypeScript |
| [`extensions/pi-session-sync/`](extensions/pi-session-sync/) | pi extension: sync pi session transcripts across machines via a private git remote. | TypeScript |

## Layout

- **Rust** lives in [`switcher/`](switcher/) (its own `Cargo.toml` and
  [`Makefile`](switcher/Makefile); build & install with `make -C switcher install`).
- **Extensions** are a **bun workspace** (`extensions/*`) — a single
  `bun install` at the root hoists one deduped `node_modules/`.

## Tasks

Run from the repo root (see the [`Makefile`](Makefile)):

```sh
make deps         # bun install (extensions)
make build        # cargo build --release + extension deps
make install      # build + install the TUI (+ csw alias) to ~/.local/bin
make install-slim # install, then reclaim the Rust build cache
make uninstall    # remove the installed binaries
make test         # rust + extension tests
make clean        # drop target/ and node_modules/ — checkout back to a few MB
```

Everything under `target/` and `node_modules/` is regenerable and gitignored, so
`make clean` returns the checkout to a few MB.
