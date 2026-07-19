//! Bake a real, human-meaningful version into the binary at build time.
//!
//! The Cargo package version (`0.1.0`) never moves — it says nothing about which
//! build you're running, which made "is this machine up to date?" unanswerable
//! (and made trzq's "synced" misleading: source pulled but the binary stale).
//!
//! Resolution order, most-authoritative first:
//!   1. `CLAUDE_SWITCHER_VERSION` env — how nix/CI stamp the pinned release tag,
//!      since those builds see a source tarball with no git history.
//!   2. `git describe --tags --always` — the everyday `make install` path. This
//!      is *exactly* what trzq compares against as its build-freshness target,
//!      so the running binary advertises the checkout it was built from.
//!   3. the crate version — last resort in a bare tarball with neither.
//!
//! Exposed to the crate as `env!("CLAUDE_SWITCHER_VERSION_STR")` via rustc-env.

use std::process::Command;

fn main() {
    // Re-stamp when the override changes or the git position moves. Best-effort:
    // these paths are absent in a tarball build, which harmlessly does nothing.
    println!("cargo:rerun-if-env-changed=CLAUDE_SWITCHER_VERSION");
    for p in [".git/HEAD", "../.git/HEAD", "../.git/refs/tags"] {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rustc-env=CLAUDE_SWITCHER_VERSION_STR={}", resolve_version());
}

fn resolve_version() -> String {
    if let Ok(v) = std::env::var("CLAUDE_SWITCHER_VERSION") {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }
    if let Some(v) = git_describe() {
        return v;
    }
    format!("v{}", std::env::var("CARGO_PKG_VERSION").unwrap_or_default())
}

fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--always"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8(out.stdout).ok()?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}
