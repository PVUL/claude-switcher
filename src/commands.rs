//! Non-interactive command implementations. Each returns a `Result` so `main`
//! can print errors uniformly.

use std::io::Write;

use chrono::{DateTime, Utc};

use crate::cli::Command;
use crate::error::Result;
use crate::manager::Manager;
use crate::paths::ACTIVE_LINK;
use crate::profile::Profile;

pub fn run(cmd: Command, mgr: &mut Manager) -> Result<()> {
    match cmd {
        Command::Switch { name } => {
            mgr.switch(&name)?;
            println!("Switched to '{name}'.");
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
        Command::Rename { old, new } => {
            mgr.rename(&old, &new)?;
            println!("Renamed '{old}' to '{new}'.");
        }
        Command::Current => match mgr.active() {
            Some(p) => println!("{}", describe(&p)),
            None => println!("(no active profile)"),
        },
        Command::List { json } => list(mgr, json)?,
        Command::Env => print_env(mgr),
    }
    Ok(())
}

fn list(mgr: &Manager, json: bool) -> Result<()> {
    let profiles = mgr.profiles();
    if json {
        let stdout = std::io::stdout();
        let mut w = stdout.lock();
        writeln!(w, "{}", profiles_to_json(&profiles))?;
        return Ok(());
    }
    if profiles.is_empty() {
        println!("No profiles yet. Add one with:  claudesub add <name>");
        return Ok(());
    }
    for p in &profiles {
        let marker = if p.active { "*" } else { " " };
        let mut line = format!("{marker} {}", p.name);
        if let Some(email) = &p.email {
            line.push_str(&format!(" ({email})"));
        }
        println!("{line}");
        println!("      path:          {}", p.raw_path);
        println!("      last used:     {}", humanize(p.last_used));
        println!("      directory:     {}", if p.exists { "present" } else { "MISSING" });
        println!("      authenticated: {}", if p.authenticated { "yes" } else { "no" });
    }
    Ok(())
}

fn describe(p: &Profile) -> String {
    match &p.email {
        Some(email) => format!("{} ({email})", p.name),
        None => p.name.clone(),
    }
}

fn print_env(mgr: &Manager) {
    let link = mgr.paths().active_link();
    println!("# Add this to your shell profile so every tool follows claudesub:");
    println!("export CLAUDE_CONFIG_DIR=\"$HOME/{ACTIVE_LINK}\"");
    let _ = link; // link already reflected via ACTIVE_LINK relative to $HOME
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
}
