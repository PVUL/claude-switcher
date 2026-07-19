//! Command-line surface, defined with clap's derive API.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "claude-switcher",
    version = crate::VERSION,
    about = "Switch between multiple isolated Claude Code accounts.",
    long_about = "claude-switcher manages several Claude Code configuration directories and \
selects one with an atomic symlink at ~/.claude-switcher. Point CLAUDE_CONFIG_DIR (or the \
bundled `claude-switcher-exec` wrapper) at that symlink and every tool follows the active profile.\n\n\
Run with no arguments to open the interactive TUI."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Switch the active profile.
    Switch {
        /// Profile name to activate.
        name: String,
    },
    /// Add a new profile (creates its directory if missing).
    Add {
        /// Profile name.
        name: String,
        /// Use a custom directory instead of ~/.claude-<name>.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Remove a profile from management.
    Remove {
        /// Profile name.
        name: String,
        /// Also delete the profile's directory from disk.
        #[arg(long)]
        purge: bool,
    },
    /// Adopt an existing Claude config directory as a profile (no copying).
    Adopt {
        /// Profile name. Defaults to a name derived from the directory
        /// (e.g. ~/.claude -> "default", ~/.claude-work -> "work").
        name: Option<String>,
        /// Directory to adopt. Defaults to ~/.claude (the standard config dir).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Auto-discover and adopt every un-managed ~/.claude[-*] directory.
        #[arg(long)]
        scan: bool,
        /// Make the adopted profile active (single-profile mode only).
        #[arg(long)]
        activate: bool,
        /// Also import login state from ~/.claude.json into the profile dir
        /// (only needed when adopting the default ~/.claude; copies, never moves).
        #[arg(long)]
        migrate_state: bool,
    },
    /// Rename a profile (moves its directory if at the default location).
    Rename {
        /// Current profile name.
        old: String,
        /// New profile name.
        new: String,
    },
    /// Print the active profile.
    Current,
    /// Show per-account usage limits (queries the Anthropic usage endpoint).
    Usage {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List all profiles.
    List {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print shell setup for pointing CLAUDE_CONFIG_DIR at the active profile.
    Env,
    /// Print shell integration for live, in-shell switching (no new terminal).
    ///
    /// Add `eval "$(claude-switcher shellenv)"` to your shell profile. It wraps
    /// the `claude-switcher` command so that, after every switch, it re-resolves
    /// and re-exports CLAUDE_CONFIG_DIR in the CURRENT shell.
    Shellenv,
    /// Diagnose and repair your setup: adopt existing Claude dirs, activate a
    /// profile, and check sign-in + usage. Applies safe fixes automatically and
    /// guides the rest. Also runs on launch when setup looks incomplete.
    Doctor {
        /// Apply all suggested fixes without prompting.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}
