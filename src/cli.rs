//! Command-line surface, defined with clap's derive API.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "claudesub",
    version,
    about = "Switch between multiple isolated Claude Code accounts.",
    long_about = "claudesub manages several Claude Code configuration directories and \
selects one with an atomic symlink at ~/.claude-active. Point CLAUDE_CONFIG_DIR (or the \
bundled `claude-active` wrapper) at that symlink and every tool follows the active profile.\n\n\
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
    /// Rename a profile (moves its directory if at the default location).
    Rename {
        /// Current profile name.
        old: String,
        /// New profile name.
        new: String,
    },
    /// Print the active profile.
    Current,
    /// List all profiles.
    List {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print shell setup for pointing CLAUDE_CONFIG_DIR at the active profile.
    Env,
}
