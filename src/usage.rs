//! Best-effort usage-limit lookup for a profile.
//!
//! This is the one place the tool talks to the network, and it is entirely
//! opt-in (the `usage` command and the TUI). Everything degrades gracefully:
//! if the token can't be found or the request fails, we simply report that
//! usage is unavailable — the core switching never depends on it.
//!
//! How it works (reverse-engineered from Claude Code itself):
//!   * The OAuth access token for a config dir is stored either in a
//!     `<dir>/.credentials.json` file (Linux) or the macOS Keychain under the
//!     service `Claude Code-credentials` for the default `~/.claude`, or
//!     `Claude Code-credentials-<first 8 hex of sha256(abs config dir)>` for any
//!     other config directory.
//!   * Usage is a GET to `https://api.anthropic.com/api/oauth/usage` with the
//!     token as a bearer and the `anthropic-beta: oauth-2025-04-20` header.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Duration, DurationRound, Local, Utc};
use serde::{Deserialize, Serialize};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-code/2.0.32";

/// A single rate-limit window (e.g. the rolling 5-hour or 7-day limit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// Percent of the limit consumed (0–100).
    pub utilization: f64,
    #[serde(rename = "resetsAt")]
    pub resets_at: Option<DateTime<Utc>>,
}

/// Usage limits for an account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(rename = "fiveHour", default)]
    pub five_hour: Option<Window>,
    #[serde(rename = "sevenDay", default)]
    pub seven_day: Option<Window>,
    #[serde(rename = "sevenDayOpus", default)]
    pub seven_day_opus: Option<Window>,
}

/// A persisted snapshot of usage for all profiles, with the time it was fetched
/// so a later session can reuse it while still fresh instead of re-querying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageCache {
    #[serde(rename = "fetchedAt")]
    pub fetched_at: DateTime<Utc>,
    #[serde(default)]
    pub profiles: HashMap<String, Usage>,
}

/// Fetch usage for the profile at `profile_path`. `active_link` should be the
/// active symlink path when this profile is the active one (Claude may key the
/// token by the path it was launched with). Returns `None` if we can't get a
/// token or the request fails.
pub fn fetch(profile_path: &Path, home: &Path, active_link: Option<&Path>) -> Option<Usage> {
    let token = access_token(profile_path, home, active_link)?;
    call_api(&token)
}

/// Human phrasing of when a window resets, e.g. "resets in 2h 5m".
pub fn resets_in(window: &Window) -> Option<String> {
    let resets_at = window.resets_at?;
    let secs = resets_at.signed_duration_since(Utc::now()).num_seconds();
    if secs <= 0 {
        return Some("resetting".to_string());
    }
    let out = if secs >= 86_400 {
        format!("resets in {}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    } else if secs >= 3600 {
        format!("resets in {}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("resets in {}m", secs / 60)
    };
    Some(out)
}

/// The reset moment as a local wall-clock time, e.g. "14:50" (today),
/// "Sat 22:00" (this week), or "Jul 08 10:00".
pub fn reset_clock(window: &Window) -> Option<String> {
    // Round to the nearest minute (rather than flooring on display) so the
    // clock time lines up with the actual reset moment.
    let resets_at = window.resets_at?;
    let local = resets_at
        .duration_round(Duration::minutes(1))
        .unwrap_or(resets_at)
        .with_timezone(&Local);
    let now = Local::now();
    // American 12-hour clock, e.g. "3:49pm", "Sun 5:59am", "Jul 8 5:59pm".
    let fmt = if local.date_naive() == now.date_naive() {
        local.format("%-I:%M%P").to_string()
    } else if local.signed_duration_since(now).num_days() < 6 {
        local.format("%a %-I:%M%P").to_string()
    } else {
        local.format("%b %-d %-I:%M%P").to_string()
    };
    Some(fmt)
}

/// A fixed-width text progress bar, e.g. "████░░░░░░" for 40% at width 10.
pub fn bar(utilization: f64, width: usize) -> String {
    let pct = utilization.clamp(0.0, 100.0);
    let filled = (((pct / 100.0) * width as f64).round() as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

// ---- token resolution -----------------------------------------------------

/// A candidate credential with its expiry, so we can prefer the freshest one.
struct Cred {
    access_token: String,
    /// Epoch-millis expiry, if known.
    expires_at: Option<i64>,
}

/// Resolve the access token for a profile.
///
/// A profile's own account-specific credentials (the on-disk token and the
/// Keychain slot(s) keyed by its *directory*) are tried first, freshest expiry
/// winning among them. The active symlink's Keychain slot is consulted **only**
/// as a last resort: it is keyed by the symlink path, not the account, so it is
/// SHARED across profiles and holds whichever account last authenticated
/// through it. Merging it into the freshest-expiry race (the previous behavior)
/// let a token left by a previously-active account leak in, making the active
/// profile report another account's usage — so we never do that anymore.
fn access_token(profile_path: &Path, home: &Path, active_link: Option<&Path>) -> Option<String> {
    let mut own: Vec<Cred> = Vec::new();
    // Linux (and any install that writes the token to disk).
    if let Some(c) = token_from_file(profile_path) {
        own.push(c);
    }
    // macOS Keychain, the profile's own directory-keyed slot(s).
    #[cfg(target_os = "macos")]
    for service in own_keychain_services(profile_path, home) {
        if let Some(c) = token_from_keychain(&service) {
            own.push(c);
        }
    }
    // Freshest expiry wins; unknown expiry is treated as oldest.
    if let Some(tok) = own
        .into_iter()
        .max_by_key(|c| c.expires_at.unwrap_or(i64::MIN))
        .map(|c| c.access_token)
    {
        return Some(tok);
    }

    // Legacy fallback only: a token stored under the unresolved symlink path.
    #[cfg(target_os = "macos")]
    if let Some(link) = active_link {
        if let Some(c) = token_from_keychain(&service_name(link, home)) {
            return Some(c.access_token);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (home, active_link);

    // True last resort: an ambient CLAUDE_CODE_OAUTH_TOKEN (headless installs,
    // CI, the pi box). It only yields usage if it happens to carry the
    // user:profile scope; a plain `claude setup-token` does not, and the
    // endpoint's scope error is handled upstream (reported as unavailable).
    env_token()
}

/// The ambient coding token, if any (`CLAUDE_CODE_OAUTH_TOKEN`).
fn env_token() -> Option<String> {
    match std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        Ok(t) if !t.is_empty() => Some(t),
        _ => None,
    }
}

/// Classification of the ambient `CLAUDE_CODE_OAUTH_TOKEN`, for `doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvToken {
    /// No `CLAUDE_CODE_OAUTH_TOKEN` in the environment.
    Absent,
    /// Present and accepted by the usage endpoint.
    Usable,
    /// Present but rejected for lacking the `user:profile` scope — a coding-only
    /// `claude setup-token`. It can drive Claude Code but not usage/profile, so
    /// claude-switcher's usage + "authenticated" display need a real login.
    ScopeLimited,
    /// Present but unusable for another reason (expired token, network error).
    Unusable,
}

/// Probe the ambient token against the usage endpoint and classify it. Does one
/// network call; only used by `doctor` (never on the hot path).
pub fn classify_env_token() -> EnvToken {
    let Some(token) = env_token() else {
        return EnvToken::Absent;
    };
    match raw_call(&token) {
        None => EnvToken::Unusable,
        Some(body) => match serde_json::from_slice::<serde_json::Value>(&body) {
            Ok(v) => {
                let msg = v
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                if !msg.is_empty() {
                    if msg.contains("scope") {
                        EnvToken::ScopeLimited
                    } else {
                        EnvToken::Unusable
                    }
                } else if parse_usage(&body).is_some() {
                    EnvToken::Usable
                } else {
                    EnvToken::Unusable
                }
            }
            Err(_) => EnvToken::Unusable,
        },
    }
}

fn token_from_file(dir: &Path) -> Option<Cred> {
    let text = std::fs::read_to_string(dir.join(".credentials.json")).ok()?;
    parse_creds(&text)
}

/// The profile's own account-specific Keychain service names (keyed by its
/// directory, both resolved and unresolved). Deliberately excludes the shared
/// active-symlink slot, which is not account-specific.
fn own_keychain_services(profile_path: &Path, home: &Path) -> Vec<String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let resolved =
        std::fs::canonicalize(profile_path).unwrap_or_else(|_| profile_path.to_path_buf());
    paths.push(resolved.clone());
    if resolved != profile_path {
        paths.push(profile_path.to_path_buf());
    }

    let mut services: Vec<String> = paths.iter().map(|p| service_name(p, home)).collect();
    services.dedup();
    services
}

fn service_name(path: &Path, home: &Path) -> String {
    if path == home.join(".claude") {
        "Claude Code-credentials".to_string()
    } else {
        format!("Claude Code-credentials-{}", config_hash(path))
    }
}

/// First 8 hex chars of `sha256(path)` — Claude Code's per-config-dir suffix.
fn config_hash(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

#[cfg(target_os = "macos")]
fn token_from_keychain(service: &str) -> Option<Cred> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_creds(&String::from_utf8_lossy(&output.stdout))
}

/// Extract the access token and expiry from a credentials JSON blob. Handles
/// both the `{"claudeAiOauth": {...}}` (Keychain) and flat shapes.
fn parse_creds(text: &str) -> Option<Cred> {
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let obj = value.get("claudeAiOauth").unwrap_or(&value);
    let access_token = obj.get("accessToken")?.as_str()?.to_string();
    let expires_at = obj.get("expiresAt").and_then(|v| v.as_i64());
    Some(Cred {
        access_token,
        expires_at,
    })
}

// ---- API call -------------------------------------------------------------

#[derive(Deserialize)]
struct RawWindow {
    utilization: f64,
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct RawUsage {
    five_hour: Option<RawWindow>,
    seven_day: Option<RawWindow>,
    seven_day_opus: Option<RawWindow>,
}

fn call_api(token: &str) -> Option<Usage> {
    parse_usage(&raw_call(token)?)
}

/// Raw GET to the usage endpoint, returning the response body on a successful
/// transfer. Callers decide what the body means ([`parse_usage`] for real data,
/// [`classify_env_token`] for diagnostics).
fn raw_call(token: &str) -> Option<Vec<u8>> {
    let output = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "6",
            USAGE_URL,
            "-H",
            "Accept: application/json",
            "-H",
            &format!("anthropic-beta: {OAUTH_BETA}"),
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-H",
            &format!("Authorization: Bearer {token}"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

/// Turn a usage-endpoint response body into a `Usage`, or `None` if it isn't a
/// usage payload.
///
/// The endpoint answers HTTP errors (expired/invalid token, rate limiting)
/// with a `{"type":"error",...}` body and a 200-ish status curl treats as
/// success. Because every `RawUsage` field is optional, such a body would
/// otherwise deserialize into an all-`None` `Usage` — a "present but empty"
/// snapshot that renders as "n/a" and gets cached, hiding the real problem
/// (a dead token) behind a confusing display for a whole poll interval. So we
/// reject any error payload, and any body with no recognizable window at all,
/// and report the profile as unavailable instead.
fn parse_usage(body: &[u8]) -> Option<Usage> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    if value.get("error").is_some() || value.get("type").and_then(|t| t.as_str()) == Some("error") {
        return None;
    }
    let raw: RawUsage = serde_json::from_value(value).ok()?;
    let usage = Usage {
        five_hour: raw.five_hour.map(Window::from),
        seven_day: raw.seven_day.map(Window::from),
        seven_day_opus: raw.seven_day_opus.map(Window::from),
    };
    // A body that parsed but carries no window is not a usable usage response
    // (e.g. an unexpected/empty payload); treat it as unavailable, not empty.
    if usage.five_hour.is_none() && usage.seven_day.is_none() && usage.seven_day_opus.is_none() {
        return None;
    }
    Some(usage)
}

impl From<RawWindow> for Window {
    fn from(raw: RawWindow) -> Self {
        Window {
            utilization: raw.utilization,
            resets_at: raw
                .resets_at
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_hash_matches_claude_code() {
        // Verified against a real Keychain entry created by Claude Code.
        assert_eq!(
            config_hash(Path::new("/Users/pyun/.claude-takeyoung")),
            "fd08061c"
        );
    }

    #[test]
    fn default_dir_uses_plain_service_name() {
        let home = Path::new("/Users/pyun");
        assert_eq!(
            service_name(&home.join(".claude"), home),
            "Claude Code-credentials"
        );
        assert_eq!(
            service_name(&home.join(".claude-work"), home),
            format!(
                "Claude Code-credentials-{}",
                config_hash(&home.join(".claude-work"))
            )
        );
    }

    #[test]
    fn own_services_exclude_the_shared_symlink_slot() {
        // The profile's own directory-keyed slot is included, but the shared
        // active-symlink slot (keyed by ~/.claude-switcher, not the account)
        // must not be — that was the cross-account usage leak.
        let home = Path::new("/Users/pyun");
        let profile = home.join(".claude-work");
        let link = home.join(".claude-switcher");
        let services = own_keychain_services(&profile, home);
        assert!(services.contains(&service_name(&profile, home)));
        assert!(!services.contains(&service_name(&link, home)));
    }

    #[test]
    fn bar_fills_proportionally() {
        assert_eq!(bar(0.0, 10), "░░░░░░░░░░");
        assert_eq!(bar(50.0, 10), "█████░░░░░");
        assert_eq!(bar(100.0, 10), "██████████");
        assert_eq!(bar(150.0, 10), "██████████"); // clamps
    }

    #[test]
    fn parses_creds_from_both_shapes() {
        let c = parse_creds(r#"{"claudeAiOauth":{"accessToken":"abc","expiresAt":1783162428812}}"#)
            .unwrap();
        assert_eq!(c.access_token, "abc");
        assert_eq!(c.expires_at, Some(1783162428812));

        let flat = parse_creds(r#"{"accessToken":"xyz"}"#).unwrap();
        assert_eq!(flat.access_token, "xyz");
        assert_eq!(flat.expires_at, None);

        assert!(parse_creds("not json").is_none());
    }

    #[test]
    fn rejects_error_bodies_instead_of_reporting_empty_usage() {
        // An expired/invalid token yields this shape with a status curl treats
        // as success. It must be unavailable, not a cacheable all-n/a Usage.
        let err = br#"{"type":"error","error":{"type":"authentication_error","message":"Invalid bearer token"}}"#;
        assert!(parse_usage(err).is_none());

        // A body with no recognizable window is likewise unavailable.
        assert!(parse_usage(b"{}").is_none());
        assert!(parse_usage(b"not json").is_none());
    }

    #[test]
    fn parses_a_real_usage_body() {
        let body = br#"{"five_hour":{"utilization":13.0,"resets_at":"2026-07-04T16:00:00Z"},"seven_day":{"utilization":11.0,"resets_at":null},"seven_day_opus":null}"#;
        let u = parse_usage(body).unwrap();
        assert_eq!(u.five_hour.unwrap().utilization, 13.0);
        assert_eq!(u.seven_day.unwrap().utilization, 11.0);
        assert!(u.seven_day_opus.is_none());
    }
}
