# claude-switcher monorepo — top-level tasks that fan out to each project.
# Rust TUI in switcher/; pi extensions (TypeScript, bun workspaces) in extensions/.

.DEFAULT_GOAL := help
BUN := bun

.PHONY: help deps build install install-keep install-slim slim uninstall test check clean install-hooks

help: ## show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | \
	  awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-9s\033[0m %s\n", $$1, $$2}'

deps: ## install extension deps (bun workspaces → one hoisted ./node_modules)
	$(BUN) install

build: deps install-hooks ## cargo build --release (the TUI) + extension deps
	cd switcher && cargo build --release

install-hooks: ## wire git to the committed hooks (auto-slim target/ on a clean commit)
	@git config core.hooksPath .githooks
	@echo "git hooks active: post-commit reclaims switcher/target when the tree is clean"

install: ## build + install the TUI (+ csw alias) to ~/.local/bin, then reclaim the build cache
	$(MAKE) -C switcher install

install-keep: ## install but KEEP the build cache (fast dev iteration)
	$(MAKE) -C switcher install-keep

install-slim: ## alias for install (kept for muscle memory — install already slims)
	$(MAKE) -C switcher install-slim

slim: ## reclaim the Rust build cache (switcher/target)
	@./scripts/slim

uninstall: ## remove the installed binaries
	$(MAKE) -C switcher uninstall

test: ## rust tests + each extension's test script
	cd switcher && cargo test
	$(BUN) run --filter '*' test

check: ## typecheck the extensions
	$(BUN) run --filter '*' check

clean: ## drop build caches (target/, node_modules/) — checkout back to a few MB
	cd switcher && cargo clean
	rm -rf node_modules extensions/*/node_modules
