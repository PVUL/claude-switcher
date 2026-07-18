# Makefile — build and install claude-switcher + the claude-switcher-exec wrapper.
#
# Usage:
#   make install                    # install to ~/.local/bin + reclaim the build cache
#   make install PREFIX=/usr/local  # install to /usr/local/bin
#   make install-keep               # install but KEEP the build cache (fast dev iteration)
#   make build                      # just build the release binary
#   make slim                       # reclaim the ~600 MB Rust build cache
#   make uninstall                  # remove the installed binaries
#
# Re-run on any machine that has Rust installed to reproduce the setup.

PREFIX ?= $(HOME)/.local
BIN_DIR := $(PREFIX)/bin
REPO_DIR := $(CURDIR)

.PHONY: all build install install-keep install-slim slim uninstall

all: build

build:
	@command -v cargo >/dev/null 2>&1 || { \
		echo "error: cargo (Rust) is required. Install from https://rustup.rs" >&2; \
		exit 1; \
	}
	@echo "==> Building claude-switcher (release)"
	@cargo build --release

# Install the standalone binary + wrapper, leaving the build cache in place —
# the fast path when you're iterating and will rebuild again shortly.
install-keep: build
	@echo "==> Installing to $(BIN_DIR)"
	@mkdir -p "$(BIN_DIR)"
	@install -m 0755 "$(REPO_DIR)/target/release/claude-switcher" "$(BIN_DIR)/claude-switcher"
	@install -m 0755 "$(REPO_DIR)/scripts/claude-switcher-exec" "$(BIN_DIR)/claude-switcher-exec"
	@ln -sf "claude-switcher" "$(BIN_DIR)/csw"
	@echo
	@echo "Installed:"
	@echo "  $(BIN_DIR)/claude-switcher"
	@echo "  $(BIN_DIR)/csw -> claude-switcher"
	@echo "  $(BIN_DIR)/claude-switcher-exec"
	@echo
	@case ":$(PATH):" in \
		*":$(BIN_DIR):"*) ;; \
		*) echo "note: add $(BIN_DIR) to your PATH:"; \
		   echo "      export PATH=\"$(BIN_DIR):\$$PATH\"" ;; \
	esac
	@echo
	@echo "Next steps"
	@echo "----------"
	@echo "1. Add your accounts (each is a full, isolated Claude config dir):"
	@echo "     claude-switcher add work"
	@echo "     claude-switcher add personal"
	@echo "   Sign in to each by running \`claude-switcher switch <name>\` then \`claude\`."
	@echo
	@echo "2. Point the Claude CLI at the active profile by adding this to your shell"
	@echo "   profile (this is what makes a plain \`claude\` follow your switches):"
	@echo "     export CLAUDE_CONFIG_DIR=\"\$$HOME/.claude-switcher\""
	@echo "   For tools that need an executable path (e.g. pi-claude-bridge), point them"
	@echo "   at the installed \`claude-switcher-exec\` instead."
	@echo
	@echo "3. Switch anytime:"
	@echo "     claude-switcher            # interactive TUI"
	@echo "     claude-switcher switch work"

# Default install = install-keep + slim. You install to *use* the tool, not to
# iterate: the standalone binary in $(BIN_DIR) doesn't need target/, and leaving
# it just balloons the checkout by ~600 MB. Use `make install-keep` to install
# without slimming while developing.
install: install-keep
	@$(MAKE) slim

# Reclaim the Rust build cache (target/, ~600 MB). Safe: the installed binary
# is a standalone copy in $(BIN_DIR) — cleaning only affects the speed of the
# NEXT build, never the installed tool's behavior or performance. Rebuild with
# `make build` (or `make install`).
slim:
	@echo "==> Reclaiming build cache (cargo clean)"
	@cargo clean
	@echo "Done — target/ cleared. Rebuild anytime with 'make build'."

# Backward-compatible alias — `install` already reclaims the cache.
install-slim: install

uninstall:
	@echo "==> Removing from $(BIN_DIR)"
	@rm -f "$(BIN_DIR)/claude-switcher" "$(BIN_DIR)/csw" "$(BIN_DIR)/claude-switcher-exec"
	@echo "Removed claude-switcher, csw and claude-switcher-exec"
