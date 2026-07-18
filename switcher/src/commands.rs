//! Non-interactive command implementations. Each returns a `Result` so `main`
//! can print errors uniformly.

use std::io::Write;

use chrono::{DateTime, Utc};

use std::path::PathBuf;

use crate::cli::Command;
use crate::error::Result;
use crate::manager::Manager;
use crate::paths::ACTIVE_LINK;
use crate::profile::Profile;

pub fn run(cmd: Command, mgr: &mut Manager) -> Result<()> {
    match cmd {
        Command::Switch { name } => {
            mgr.switch(&name)?;
            println!("Switched {name} — relaunch Claude in this terminal or start new session.");
        }
        Command::Add { name, path } => {
            let profile = mgr.add(&name, path.as_deref())?;
            print!("Added profile '{name}' at {}", profile.raw_path);
            if profile.active {
                print!(" (now active)");
            }
            println!(".");
        }
        Command::Remove { name, purge } => {
            mgr.remove(&name, purge)?;
            if purge {
                println!("Removed profile '{name}' and deleted its directory.");
            } else {
                println!("Removed profile '{name}' (directory left on disk).");
            }
        }
        Command::Adopt {
            name,
            path,
            scan,
            activate,
            migrate_state,
        } => adopt(mgr, name, path, scan, activate, migrate_state)?,
        Command::Rename { old, new } => {
            mgr.rename(&old, &new)?;
            println!("Renamed '{old}' to '{new}'.");
        }
        Command::Current => match mgr.profiles_reported().into_iter().find(|p| p.active) {
            Some(p) => println!("{}", describe(&p)),
            None => println!("(no active profile)"),
        },
        Command::List { json } => list(mgr, json)?,
        Command::Usage { json } => usage(mgr, json)?,
        Command::Env => print_env(mgr),
        Command::Shellenv => print!("{}", shellenv_script()),
        Command::Doctor { yes } => crate::doctor::run(mgr, yes)?,
    }
    Ok(())
}

fn adopt(
    mgr: &mut Manager,
    name: Option<String>,
    path: Option<PathBuf>,
    scan: bool,
    activate: bool,
    migrate_state: bool,
) -> Result<()> {
    if scan {
        let candidates = mgr.discover_candidates();
        if candidates.is_empty() {
            println!("No un-managed Claude config directories found.");
            return Ok(());
        }
        for (name, path) in candidates {
            let profile = mgr.adopt(&name, &path, false)?;
            print!("Adopted '{name}' from {}", profile.raw_path);
            if profile.active {
                print!(" (now active)");
            }
            println!(".");
            maybe_migrate(mgr, &name, &profile, migrate_state);
        }
        return Ok(());
    }

    let path = path.unwrap_or_else(|| mgr.paths().home.join(".claude"));
    let name = name.unwrap_or_else(|| derive_default_name(&path));
    let profile = mgr.adopt(&name, &path, activate)?;
    print!("Adopted '{name}' from {}", profile.raw_path);
    if profile.active {
        print!(" (now active)");
    }
    println!(".");
    maybe_migrate(mgr, &name, &profile, migrate_state);
    Ok(())
}

fn maybe_migrate(mgr: &Manager, name: &str, profile: &Profile, migrate_state: bool) {
    if migrate_state {
        match mgr.migrate_home_state(name) {
            Ok(true) => println!("  imported login state from ~/.claude.json"),
            Ok(false) => {}
            Err(e) => eprintln!("  warning: could not import login state: {e}"),
        }
    } else if !profile.authenticated
        && mgr.paths().home.join(".claude.json").is_file()
        && !profile.path.join(".claude.json").exists()
    {
        println!("  note: this profile has no login state yet. Re-run with --migrate-state");
        println!("        to import ~/.claude.json, or sign in via `claude-switcher-exec`.");
    }
}

fn derive_default_name(path: &std::path::Path) -> String {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(".claude") => "default".to_string(),
        Some(n) => n.trim_start_matches(".claude-").trim_start_matches('.').to_string(),
        None => "default".to_string(),
    }
}

fn list(mgr: &Manager, json: bool) -> Result<()> {
    // Reported (pin-aware): in a session pinned via CLAUDE_SWITCHER_PIN, mark the
    // pinned account active so the listing matches what the session runs on.
    let profiles = mgr.profiles_reported();
    if json {
        let stdout = std::io::stdout();
        let mut w = stdout.lock();
        writeln!(w, "{}", profiles_to_json(&profiles))?;
        return Ok(());
    }
    if profiles.is_empty() {
        println!("No profiles yet. Add one with:  claude-switcher add <name>");
        return Ok(());
    }
    for p in &profiles {
        let marker = if p.active { "*" } else { " " };
        let mut line = format!("{marker} {}", p.name);
        if let Some(id) = p.identity() {
            line.push_str(&format!(" ({id})"));
        }
        println!("{line}");
        println!("      path:          {}", p.raw_path);
        println!("      last used:     {}", humanize(p.last_used));
        println!("      directory:     {}", if p.exists { "present" } else { "MISSING" });
        println!("      authenticated: {}", if p.authenticated { "yes" } else { "no" });
        if let Some((cur, exp)) = p.email_mismatch() {
            println!("      ⚠ WRONG ACCOUNT: signed in as {cur}, but this profile is {exp}");
        }
    }
    Ok(())
}

fn usage(mgr: &mut Manager, json: bool) -> Result<()> {
    let home = mgr.paths().home.clone();
    let link = mgr.paths().active_link();
    // Pin-aware for the displayed active marker...
    let profiles = mgr.profiles_reported();
    // ...but the symlink-keyed Keychain fallback belongs to the *real* symlink
    // target, which a pin does not move — so key `active_link` off that, not the
    // possibly-pinned display active.
    let symlink_active = mgr.symlink_active_name();
    if profiles.is_empty() {
        println!("No profiles yet.");
        return Ok(());
    }

    // Fallback source: last-known windows. A window's reset moment is deterministic,
    // so a cached window stays accurate until it expires — letting an offline / not-
    // recently-used account keep showing its real reset time instead of nothing.
    let cached: std::collections::HashMap<String, crate::usage::Usage> =
        mgr.usage_cache().map(|c| c.profiles.clone()).unwrap_or_default();
    let mut persist: std::collections::HashMap<String, crate::usage::Usage> =
        std::collections::HashMap::new();

    // (profile, usage-to-show, is_cached)
    let mut rows: Vec<(Profile, Option<crate::usage::Usage>, bool)> = Vec::new();
    for p in &profiles {
        let is_symlink_active = symlink_active.as_deref() == Some(p.name.as_str());
        let active_link = if is_symlink_active { Some(link.as_path()) } else { None };
        let live = if p.exists {
            crate::usage::fetch(&p.path, &home, active_link)
        } else {
            None
        };
        let (usage, is_cached) = match live {
            Some(u) => (Some(u), false),
            None => (cached.get(&p.name).cloned().and_then(|u| u.keep_unexpired()), true),
        };
        if let Some(u) = &usage {
            persist.insert(p.name.clone(), u.clone());
        }
        rows.push((p.clone(), usage, is_cached));
    }
    // Persist what we showed (fresh or still-valid cached) so reset times survive
    // across runs until they expire.
    let _ = mgr.save_usage_cache(crate::usage::UsageCache {
        fetched_at: Utc::now(),
        profiles: persist,
    });

    if json {
        let items: Vec<serde_json::Value> =
            rows.iter().map(|(p, u, c)| usage_json(p, u.as_ref(), *c)).collect();
        println!("{}", serde_json::to_string_pretty(&items).expect("serializable"));
        return Ok(());
    }

    for (p, usage, is_cached) in &rows {
        let marker = if p.active { "*" } else { " " };
        let mut header = format!("{marker} {}", p.name);
        if let Some(id) = p.identity() {
            header.push_str(&format!(" ({id})"));
        }
        println!("{header}");
        if let Some((cur, exp)) = p.email_mismatch() {
            println!("      ⚠ WRONG ACCOUNT: signed in as {cur}, but this profile is {exp}");
        }
        match usage {
            Some(u) => {
                if *is_cached {
                    println!("      (cached — not queried just now; valid until reset)");
                }
                print_window("5-hour", u.five_hour.as_ref());
                print_window("7-day", u.seven_day.as_ref());
                if let Some(w) = u.seven_day_opus.as_ref().filter(|w| w.utilization > 0.0) {
                    print_window("opus", Some(w));
                }
            }
            None => println!("      usage:  unavailable (not signed in, token expired, or offline)"),
        }
    }
    Ok(())
}

fn print_window(label: &str, window: Option<&crate::usage::Window>) {
    // A window we couldn't fetch is treated as freshly reset (0%) rather than
    // "n/a" — a missing window means no usage on record.
    let util = window.map_or(0.0, |w| w.utilization);
    let bar = crate::usage::bar(util, 20);
    let pct = util.round() as i64;
    let reset = window.map(reset_phrase).unwrap_or_default();
    println!("      {label:<7} [{bar}] {pct:>3}%   {reset}");
}

/// Combine relative and absolute reset info, e.g. "resets in 3h 36m (14:50)".
fn reset_phrase(window: &crate::usage::Window) -> String {
    match (crate::usage::resets_in(window), crate::usage::reset_clock(window)) {
        (Some(rel), Some(clock)) => format!("{rel} ({clock})"),
        (Some(rel), None) => rel,
        _ => String::new(),
    }
}

fn usage_json(p: &Profile, usage: Option<&crate::usage::Usage>, cached: bool) -> serde_json::Value {
    let win = |w: Option<&crate::usage::Window>| {
        w.map(|w| {
            serde_json::json!({
                "utilization": w.utilization,
                "resetsAt": w.resets_at,
            })
        })
    };
    serde_json::json!({
        "name": p.name,
        "email": p.email,
        "expectedEmail": p.expected_email,
        "emailMismatch": p.email_mismatch().is_some(),
        "plan": p.plan,
        "active": p.active,
        "available": usage.is_some(),
        "cached": cached && usage.is_some(),
        "fiveHour": usage.and_then(|u| win(u.five_hour.as_ref())),
        "sevenDay": usage.and_then(|u| win(u.seven_day.as_ref())),
        "sevenDayOpus": usage.and_then(|u| win(u.seven_day_opus.as_ref())),
    })
}

fn describe(p: &Profile) -> String {
    let base = match p.identity() {
        Some(id) => format!("{} ({id})", p.name),
        None => p.name.clone(),
    };
    match p.email_mismatch() {
        Some((cur, exp)) => format!("{base}  ⚠ signed in as {cur}, expected {exp}"),
        None => base,
    }
}

fn print_env(mgr: &Manager) {
    let link = mgr.paths().active_link();
    println!("# Add this to your shell profile so every tool follows claude-switcher:");
    println!("export CLAUDE_CONFIG_DIR=\"$HOME/{ACTIVE_LINK}\"");
    println!("# For live switching without a new terminal, use `shellenv` instead:");
    println!("#   eval \"$(claude-switcher shellenv)\"");
    let _ = link; // link already reflected via ACTIVE_LINK relative to $HOME
}

/// Shell integration that keeps CLAUDE_CONFIG_DIR in sync with the active
/// symlink for the *current* shell, so `claude-switcher switch` (or a switch
/// made inside the TUI) takes effect without opening a new terminal.
///
/// We export the RESOLVED symlink target rather than the symlink path itself:
/// macOS Claude Code keys its Keychain OAuth token by a hash of the literal
/// CLAUDE_CONFIG_DIR string, so the unresolved path would make every profile
/// share one token slot. A wrapper function shadows the binary and re-resolves
/// after each invocation; `command claude-switcher` calls the real executable.
fn shellenv_script() -> String {
    format!(
        r#"# claude-switcher shell integration.
# Add to your shell profile:  eval "$(claude-switcher shellenv)"
__claude_switcher_sync() {{
  export CLAUDE_CONFIG_DIR="$(readlink "$HOME/{ACTIVE_LINK}" 2>/dev/null || echo "$HOME/{ACTIVE_LINK}")"
}}
__claude_switcher_sync
claude-switcher() {{
  command claude-switcher "$@"
  local __cs_status=$?
  __claude_switcher_sync
  return $__cs_status
}}
"#
    )
}

/// Human-friendly relative time, matching the TUI's wording.
pub fn humanize(ts: Option<DateTime<Utc>>) -> String {
    let Some(ts) = ts else {
        return "never".to_string();
    };
    let delta = Utc::now().signed_duration_since(ts);
    let secs = delta.num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    match secs {
        s if s < 60 => "just now".to_string(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} hr ago", s / 3600),
        s if s < 172_800 => "yesterday".to_string(),
        s => format!("{} days ago", s / 86_400),
    }
}

fn profiles_to_json(profiles: &[Profile]) -> String {
    let items: Vec<serde_json::Value> = profiles
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "path": p.raw_path,
                "email": p.email,
                "expectedEmail": p.expected_email,
                "emailMismatch": p.email_mismatch().is_some(),
                "plan": p.plan,
                "active": p.active,
                "exists": p.exists,
                "authenticated": p.authenticated,
                "lastUsed": p.last_used,
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).expect("serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn humanize_buckets() {
        let now = Utc::now();
        assert_eq!(humanize(None), "never");
        assert_eq!(humanize(Some(now - Duration::seconds(5))), "just now");
        assert_eq!(humanize(Some(now - Duration::minutes(2))), "2 min ago");
        assert_eq!(humanize(Some(now - Duration::hours(3))), "3 hr ago");
        assert_eq!(humanize(Some(now - Duration::hours(30))), "yesterday");
        assert_eq!(humanize(Some(now - Duration::days(4))), "4 days ago");
    }

    #[test]
    fn shellenv_wraps_the_binary_and_resyncs() {
        let script = shellenv_script();
        // Shadows the binary but calls the real one to avoid recursion.
        assert!(script.contains("claude-switcher() {"));
        assert!(script.contains("command claude-switcher \"$@\""));
        // Re-exports the RESOLVED target in the current shell after each call.
        assert!(script.contains("export CLAUDE_CONFIG_DIR="));
        assert!(script.contains(&format!("readlink \"$HOME/{ACTIVE_LINK}\"")));
        assert!(script.contains("__claude_switcher_sync"));
        // Preserves the wrapped command's exit status.
        assert!(script.contains("return $__cs_status"));
    }
}
