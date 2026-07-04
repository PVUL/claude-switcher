#!/bin/sh
# install.sh — build and install claudesub + the claude-active wrapper.
#
# Usage:
#   ./install.sh            # install to ~/.local/bin (no sudo)
#   PREFIX=/usr/local ./install.sh   # install to /usr/local/bin
#
# Re-run on any machine that has Rust installed to reproduce the setup.
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo (Rust) is required. Install from https://rustup.rs" >&2
    exit 1
fi

echo "==> Building claudesub (release)"
( cd "$REPO_DIR" && cargo build --release )

echo "==> Installing to $BIN_DIR"
mkdir -p "$BIN_DIR"
install -m 0755 "$REPO_DIR/target/release/claudesub" "$BIN_DIR/claudesub"
install -m 0755 "$REPO_DIR/scripts/claude-active" "$BIN_DIR/claude-active"

echo
echo "Installed:"
echo "  $BIN_DIR/claudesub"
echo "  $BIN_DIR/claude-active"
echo

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "note: add $BIN_DIR to your PATH:"
       echo "      export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

cat <<'EOF'

Next steps
----------
1. Add your accounts (each is a full, isolated Claude config dir):
     claudesub add work
     claudesub add personal
   Sign in to each by running `claudesub switch <name>` then `claude-active`.

2. Point every consumer at the active profile. Easiest is the alias:
     alias claude='claude-active'
   Or export the variable in your shell profile:
     export CLAUDE_CONFIG_DIR="$HOME/.claude-active"

3. Switch anytime:
     claudesub            # interactive TUI
     claudesub switch work
EOF
