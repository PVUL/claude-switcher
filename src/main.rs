//! claudesub — switch between multiple isolated Claude Code accounts.

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

use cli::Cli;
use manager::Manager;
use paths::Paths;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("claudesub: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> error::Result<()> {
    let paths = Paths::discover()?;
    let mut manager = Manager::load(paths)?;

    match cli.command {
        Some(cmd) => commands::run(cmd, &mut manager),
        None => tui::run(&mut manager),
    }
}
