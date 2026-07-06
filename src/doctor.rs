//! `claude-switcher doctor` — diagnose and repair a claude-switcher setup.
//!
//! It walks the same steps a person would when setting the tool up on a new
//! machine:
//!   1. adopt the Claude config directories that already exist here,
//!   2. make sure exactly one profile is active (the `~/.claude-switcher` link),
//!   3. check that each profile is actually signed in, and
//!   4. check that the active profile's usage endpoint is reachable.
//!
//! Safe, additive fixes (adopting an existing directory, activating the sole
//! profile) are applied automatically. Anything that needs a human — choosing
//! among several accounts, signing in through a browser — is *guided*, never
//! forced, so the tool can never sign you out or clobber an account.
//!
//! It runs two ways:
//!   * `claude-switcher doctor` always prints the full report ([`Mode::Explicit`]).
//!   * [`ensure_setup_on_launch`] runs it before the TUI: silent when everything
//!     is healthy, otherwise the same wizard. Non-interactive callers (the pi
//!     extension's `usage --json` / `list --json`, pipes) never trigger it, and
//!     `CLAUDE_SWITCHER_NO_WIZARD=1` opts out entirely.

use std::io::{self, IsTerminal, Write};

use crate::error::Result;
use crate::manager::Manager;
use crate::profile::Profile;
use crate::usage::{self, EnvToken};

/// Environment variable that suppresses the automatic launch wizard.
const NO_WIZARD_ENV: &str = "CLAUDE_SWITCHER_NO_WIZARD";

const OK: &str = "\u{2713}"; // ✓
const WARN: &str = "!";
const BAD: &str = "\u{2717}"; // ✗

/// How `doctor` was invoked, which controls how chatty it is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Explicit `claude-switcher doctor`: always print a full report.
    Explicit,
    /// Auto-run before the TUI: print only when acting or when something's wrong.
    Launch,
}

/// Explicit `claude-switcher doctor`. Always prints a full report; applies safe
/// fixes, prompting first unless `yes` is set.
pub fn run(mgr: &mut Manager, yes: bool) -> Result<()> {
    println!("claude-switcher doctor\n");
    let healthy = walk(mgr, Mode::Explicit, yes)?;
    println!();
    if healthy {
        println!("{OK} Everything looks good.");
    } else {
        println!("Some items need attention — see above.");
    }
    Ok(())
}

/// Run before the interactive TUI. Silently applies safe fixes; when setup is
/// incomplete and we're on a terminal, it engages the wizard. A no-op when
/// everything is already healthy, when not on a TTY, or when opted out.
pub fn ensure_setup_on_launch(mgr: &mut Manager) -> Result<()> {
    if std::env::var_os(NO_WIZARD_ENV).is_some() || !io::stdout().is_terminal() {
        return Ok(());
    }
    // "Unless there truly are no Claude profiles here" — nothing managed and
    // nothing to adopt means there's nothing to set up; leave it to the TUI's
    // own empty-state message.
    if mgr.profiles().is_empty() && mgr.discover_candidates().is_empty() {
        return Ok(());
    }
    walk(mgr, Mode::Launch, false)?;
    Ok(())
}

/// The shared setup walk. Returns whether the setup is fully healthy.
fn walk(mgr: &mut Manager, mode: Mode, yes: bool) -> Result<bool> {
    let mut healthy = true;

    // 1. Adopt any un-managed Claude config directories found here.
    let candidates = mgr.discover_candidates();
    if !candidates.is_empty() {
        let names: Vec<String> = candidates.iter().map(|(n, _)| n.clone()).collect();
        if yes || confirm(mode, &format!("Adopt existing Claude dir(s): {}?", names.join(", "))) {
            for (name, path) in &candidates {
                match mgr.adopt(name, path, false) {
                    Ok(p) => report(OK, &format!("adopted '{name}' ({})", p.raw_path)),
                    Err(e) => {
                        report(BAD, &format!("could not adopt '{name}': {e}"));
                        healthy = false;
                    }
                }
            }
        } else {
            // The user said no. Remember it so the wizard stops re-offering these
            // on every launch; they can still adopt one by hand later.
            let paths: Vec<_> = candidates.iter().map(|(_, p)| p.clone()).collect();
            if mgr.ignore_candidates(&paths).is_ok() {
                report(OK, &format!("skipping {} (won't ask again)", names.join(", ")));
                report(OK, "    to adopt one later: claude-switcher adopt <name> --path <dir>");
            } else {
                report(WARN, &format!("un-adopted Claude dir(s): {}", names.join(", ")));
                healthy = false;
            }
        }
    }

    let profiles = mgr.profiles();
    if profiles.is_empty() {
        report(WARN, "no profiles yet — sign in with `claude`, then re-run");
        return Ok(false);
    }

    // 2. Make sure one profile is active.
    if mgr.active().is_none() {
        if profiles.len() == 1 {
            let name = profiles[0].name.clone();
            mgr.switch(&name)?;
            report(OK, &format!("activated the only profile: '{name}'"));
        } else if let Some(name) = choose_profile(&profiles, yes) {
            mgr.switch(&name)?;
            report(OK, &format!("activated '{name}'"));
        } else {
            report(WARN, "no active profile — pick one in the TUI (enter)");
            healthy = false;
        }
    } else if mode == Mode::Explicit {
        let active = mgr.active().expect("checked");
        report(OK, &format!("active profile: {}", label(&active)));
    }

    // 3. Sign-in status per profile (filesystem-only, so cheap on every launch).
    for p in &mgr.profiles() {
        if !p.exists {
            report(BAD, &format!("'{}' directory is missing ({})", p.name, p.raw_path));
            healthy = false;
        } else if p.authenticated {
            if mode == Mode::Explicit {
                report(OK, &format!("'{}' is signed in", p.name));
            }
        } else {
            report(WARN, &format!("'{}' is not signed in", p.name));
            report(WARN, &format!("    fix: claude-switcher switch {} && claude   (then /login)", p.name));
            healthy = false;
        }
    }

    // 4. Usage reachability for the active profile, with a precise diagnosis of
    //    the common headless case (a coding-only setup-token). Explicit only:
    //    it makes a network call, which we don't want on every TUI launch (the
    //    TUI fetches usage itself, asynchronously).
    if mode == Mode::Explicit {
        if let Some(active) = mgr.active() {
            if active.exists {
                let home = mgr.paths().home.clone();
                let link = mgr.paths().active_link();
                if usage::fetch(&active.path, &home, Some(link.as_path())).is_some() {
                    report(OK, "usage endpoint reachable for the active profile");
                } else {
                    healthy = false;
                    diagnose_usage(&active.name);
                }
            }
        }
    }

    // 5. Shell integration is advisory only (never a failure).
    if mode == Mode::Explicit {
        check_shell_integration(mgr);
    }

    Ok(healthy)
}

/// Explain why usage came back unavailable for the active profile, naming the
/// coding-only setup-token case precisely so a headless box knows what to do.
fn diagnose_usage(active_name: &str) {
    match usage::classify_env_token() {
        EnvToken::ScopeLimited => {
            report(WARN, "usage unavailable: CLAUDE_CODE_OAUTH_TOKEN is a coding-only");
            report(WARN, "    setup-token (missing the user:profile scope). It runs Claude");
            report(WARN, "    fine, but usage/sign-in display needs a real interactive login:");
            report(WARN, &format!("    claude-switcher switch {active_name} && claude   (then /login)"));
        }
        EnvToken::Usable => {
            report(WARN, "usage unavailable for the active profile (sign in with `claude`)");
        }
        EnvToken::Unusable => {
            report(WARN, "usage unavailable (token expired or network unreachable)");
        }
        EnvToken::Absent => {
            report(WARN, "usage unavailable (not signed in, token expired, or offline)");
        }
    }
}

/// Ask the user to pick a profile to activate. With `yes`, activates the
/// most-recently-used; otherwise prompts, and gives up if there's no TTY.
fn choose_profile(profiles: &[Profile], yes: bool) -> Option<String> {
    if yes {
        return profiles.first().map(|p| p.name.clone());
    }
    if !io::stdin().is_terminal() {
        return None;
    }
    println!("Which profile should be active?");
    for (i, p) in profiles.iter().enumerate() {
        println!("  {}) {}", i + 1, label(p));
    }
    print!("Enter a number (or blank to skip): ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return None;
    }
    let choice: usize = line.trim().parse().ok()?;
    profiles.get(choice.checked_sub(1)?).map(|p| p.name.clone())
}

/// A yes/no prompt. In `Launch` mode, defaults to yes for the safe adoptions so
/// setup "just works"; falls back to yes when there's no TTY to ask on (the fix
/// is non-destructive).
fn confirm(mode: Mode, question: &str) -> bool {
    if !io::stdin().is_terminal() {
        return true;
    }
    let default_yes = mode == Mode::Launch;
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return default_yes;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    }
}

/// Advisory check: is CLAUDE_CONFIG_DIR wired to the active symlink?
fn check_shell_integration(mgr: &Manager) {
    let link = mgr.paths().active_link();
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(v) => {
            let set = std::path::PathBuf::from(&v);
            let target = std::fs::read_link(&link).unwrap_or_else(|_| link.clone());
            if set == link || set == target {
                report(OK, "CLAUDE_CONFIG_DIR follows the active profile");
            } else {
                report(WARN, &format!("CLAUDE_CONFIG_DIR is set to {} (not the active profile)", set.display()));
                report(WARN, "    consider: eval \"$(claude-switcher shellenv)\"");
            }
        }
        None => {
            report(WARN, "CLAUDE_CONFIG_DIR is not set — tools won't follow switches");
            report(WARN, "    add to your shell profile: eval \"$(claude-switcher shellenv)\"");
        }
    }
}

fn label(p: &Profile) -> String {
    match p.identity() {
        Some(id) => format!("{} ({id})", p.name),
        None => p.name.clone(),
    }
}

fn report(sym: &str, msg: &str) {
    println!("{sym} {msg}");
}

#[cfg(test)]
mod tests {
    // The setup walk is I/O- and network-bound (filesystem, curl, TTY prompts),
    // so it is exercised via the CLI end-to-end rather than unit-tested here.
    // Pure helpers that can be tested live in their own modules (detect, usage).

    #[test]
    fn symbols_are_single_column() {
        assert_eq!(super::OK.chars().count(), 1);
        assert_eq!(super::BAD.chars().count(), 1);
        assert_eq!(super::WARN.chars().count(), 1);
    }
}
