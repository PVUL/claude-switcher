//! Read-only inspection of a Claude configuration directory.
//!
//! We never contact Anthropic. Everything is inferred locally from the files
//! Claude Code already writes:
//!   * `<dir>/.claude.json` holds `oauthAccount.emailAddress` once signed in.
//!   * `<dir>/.credentials.json` exists on platforms that store the token on
//!     disk (Linux); on macOS the token lives in the Keychain, so the presence
//!     of `oauthAccount` is the reliable signal instead.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Files/dirs Claude touches when a profile is actually *used* (a session runs,
/// a prompt is sent, state is written). Their newest mtime is our "last used".
const ACTIVITY_PATHS: [&str; 4] = [".claude.json", "history.jsonl", "sessions", "projects"];

/// What we could learn about the account backing a profile directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub email: Option<String>,
    /// Brief plan label, e.g. "Pro", "Max 5x", "Team", "Enterprise".
    pub plan: Option<String>,
    pub authenticated: bool,
}

#[derive(Deserialize)]
struct ClaudeJson {
    #[serde(rename = "oauthAccount")]
    oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize)]
struct OauthAccount {
    #[serde(rename = "emailAddress")]
    email_address: Option<String>,
    #[serde(rename = "organizationType")]
    organization_type: Option<String>,
    #[serde(rename = "userRateLimitTier")]
    user_rate_limit_tier: Option<String>,
}

/// Derive a short plan label from the OAuth account fields.
fn plan_label(org_type: Option<&str>, rate_tier: Option<&str>) -> Option<String> {
    if let Some(org) = org_type {
        let base = org.strip_prefix("claude_").unwrap_or(org);
        match base {
            "pro" => return Some("Pro".to_string()),
            "team" => return Some("Team".to_string()),
            "enterprise" => return Some("Enterprise".to_string()),
            "max" => return Some(max_label(rate_tier)),
            _ => {}
        }
    }
    // Fall back to inferring a Max tier from the rate-limit tier.
    if let Some(r) = rate_tier {
        if r.contains("max_20x") {
            return Some("Max 20x".to_string());
        }
        if r.contains("max_5x") {
            return Some("Max 5x".to_string());
        }
    }
    // Last resort: title-case whatever organization type we saw.
    org_type.and_then(|o| {
        let base = o.strip_prefix("claude_").unwrap_or(o);
        let mut chars = base.chars();
        chars.next().map(|f| f.to_uppercase().collect::<String>() + chars.as_str())
    })
}

fn max_label(rate_tier: Option<&str>) -> String {
    match rate_tier {
        Some(r) if r.contains("20x") => "Max 20x".to_string(),
        Some(r) if r.contains("5x") => "Max 5x".to_string(),
        _ => "Max".to_string(),
    }
}

/// Inspect a profile directory. `home` is used only to cover the special case
/// of the default `~/.claude` profile, whose `.claude.json` lives at
/// `~/.claude.json` (in the home dir) rather than inside the directory.
pub fn inspect(dir: &Path, home: &Path) -> Account {
    let mut account = Account::default();

    if dir.join(".credentials.json").is_file() {
        account.authenticated = true;
    }

    for candidate in claude_json_candidates(dir, home) {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            if let Ok(parsed) = serde_json::from_str::<ClaudeJson>(&text) {
                if let Some(oauth) = parsed.oauth_account {
                    account.authenticated = true;
                    account.plan = plan_label(
                        oauth.organization_type.as_deref(),
                        oauth.user_rate_limit_tier.as_deref(),
                    );
                    if let Some(email) = oauth.email_address {
                        account.email = Some(email);
                    }
                    break;
                }
            }
        }
    }

    account
}

/// When a profile was last *used*, inferred from filesystem activity rather
/// than from when it was selected. Returns `None` if nothing has been touched.
pub fn last_used(dir: &Path, home: &Path) -> Option<DateTime<Utc>> {
    let mut candidates: Vec<PathBuf> = ACTIVITY_PATHS.iter().map(|f| dir.join(f)).collect();
    // The default ~/.claude profile keeps its state file beside the directory.
    if dir == home.join(".claude") {
        candidates.push(home.join(".claude.json"));
    }
    candidates
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .filter_map(|m| m.modified().ok())
        .map(DateTime::<Utc>::from)
        .max()
}

fn claude_json_candidates(dir: &Path, home: &Path) -> Vec<std::path::PathBuf> {
    let mut candidates = vec![dir.join(".claude.json")];
    // The default profile keeps its state file beside the directory.
    if dir == home.join(".claude") {
        candidates.push(home.join(".claude.json"));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_email_and_auth() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"paul@nhost.io"}}"#,
        )
        .unwrap();
        let account = inspect(dir.path(), dir.path());
        assert_eq!(account.email.as_deref(), Some("paul@nhost.io"));
        assert!(account.authenticated);
    }

    #[test]
    fn detects_plan() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"a@b.co","organizationType":"claude_team","userRateLimitTier":"default_claude_max_5x"}}"#,
        )
        .unwrap();
        // organizationType wins: a team member is "Team", not "Max 5x".
        assert_eq!(inspect(dir.path(), dir.path()).plan.as_deref(), Some("Team"));
    }

    #[test]
    fn plan_label_variants() {
        assert_eq!(plan_label(Some("claude_pro"), None).as_deref(), Some("Pro"));
        assert_eq!(plan_label(Some("claude_team"), None).as_deref(), Some("Team"));
        assert_eq!(
            plan_label(Some("claude_enterprise"), None).as_deref(),
            Some("Enterprise")
        );
        assert_eq!(
            plan_label(Some("claude_max"), Some("default_claude_max_20x")).as_deref(),
            Some("Max 20x")
        );
        assert_eq!(
            plan_label(Some("claude_max"), Some("default_claude_max_5x")).as_deref(),
            Some("Max 5x")
        );
        // Individual Max with no org type, inferred from the rate tier.
        assert_eq!(
            plan_label(None, Some("default_claude_max_20x")).as_deref(),
            Some("Max 20x")
        );
        assert_eq!(plan_label(None, None), None);
    }

    #[test]
    fn credentials_file_marks_authenticated() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".credentials.json"), "{}").unwrap();
        let account = inspect(dir.path(), dir.path());
        assert!(account.authenticated);
        assert_eq!(account.email, None);
    }

    #[test]
    fn empty_dir_is_unauthenticated() {
        let dir = tempdir().unwrap();
        assert_eq!(inspect(dir.path(), dir.path()), Account::default());
    }

    #[test]
    fn last_used_none_when_untouched() {
        let dir = tempdir().unwrap();
        assert_eq!(last_used(dir.path(), dir.path()), None);
    }

    #[test]
    fn last_used_reflects_activity_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("history.jsonl"), "{}").unwrap();
        assert!(last_used(dir.path(), dir.path()).is_some());
    }
}
