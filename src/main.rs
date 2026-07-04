//! claude-switcher — switch between multiple isolated Claude Code accounts.

mod cli;
mod commands;
mod detect;
mod error;
mod manager;
mod metadata;
mod paths;
mod profile;
mod symlink;
mod tui;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};
use manager::Manager;
use paths::Paths;

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

    // On first use, or for read-only commands, import any Claude config you're
    // already signed in to so the tool shows your current account right away.
    let bootstrap = matches!(
        cli.command,
        None | Some(Command::List { .. }) | Some(Command::Current)
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
        None => tui::run(&mut manager),
    }
}
