# Makefile — build and install claude-switcher + the claude-switcher-exec wrapper.
#
# Usage:
#   make install                    # install to ~/.local/bin (no sudo)
#   make install PREFIX=/usr/local  # install to /usr/local/bin
#   make build                      # just build the release binary
#   make uninstall                  # remove the installed binaries
#
# Re-run on any machine that has Rust installed to reproduce the setup.

PREFIX ?= $(HOME)/.local
BIN_DIR := $(PREFIX)/bin
REPO_DIR := $(CURDIR)

.PHONY: all build install uninstall

all: build

build:
	@command -v cargo >/dev/null 2>&1 || { \
		echo "error: cargo (Rust) is required. Install from https://rustup.rs" >&2; \
		exit 1; \
	}
	@echo "==> Building claude-switcher (release)"
	@cargo build --release

install: build
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

uninstall:
	@echo "==> Removing from $(BIN_DIR)"
	@rm -f "$(BIN_DIR)/claude-switcher" "$(BIN_DIR)/csw" "$(BIN_DIR)/claude-switcher-exec"
	@echo "Removed claude-switcher, csw and claude-switcher-exec"
