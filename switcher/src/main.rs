//! claude-switcher — switch between multiple isolated Claude Code accounts.

mod cli;
mod commands;
mod detect;
mod doctor;
mod error;
mod manager;
mod metadata;
mod paths;
mod profile;
mod symlink;
mod tui;
mod usage;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};
use manager::Manager;
use paths::Paths;

/// The running build's version, baked in by build.rs (git describe on
/// `make install`, or the pinned release tag nix passes on the box). Shown by
/// `--version` and in the TUI header so every machine can answer "which
/// version?" — and so trzq can compare the installed binary against latest.
pub const VERSION: &str = env!("CLAUDE_SWITCHER_VERSION_STR");

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("claude-switcher: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> error::Result<()> {
    let paths = Paths::discover()?;
    let mut manager = Manager::load(paths)?;

    // For read-only commands, import any Claude config you're already signed in
    // to so the tool shows your current account right away. (The interactive
    // launch path below does its own, richer setup via `doctor`.)
    let bootstrap = matches!(
        cli.command,
        Some(Command::List { .. }) | Some(Command::Current) | Some(Command::Usage { .. })
    );
    if bootstrap {
        let adopted = manager.bootstrap_if_empty()?;
        if !adopted.is_empty() {
            eprintln!(
                "claude-switcher: imported existing Claude account(s): {}",
                adopted.join(", ")
            );
        }
    }

    match cli.command {
        Some(cmd) => commands::run(cmd, &mut manager),
        None => {
            // Before the TUI, make sure setup is sane (adopt dirs, activate a
            // profile, flag sign-in/usage gaps). Silent when already healthy.
            doctor::ensure_setup_on_launch(&mut manager)?;
            tui::run(&mut manager)
        }
    }
}
