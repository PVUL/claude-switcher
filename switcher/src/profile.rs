//! The runtime view of a profile, enriched with live filesystem state.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

/// A profile as presented to the user: metadata merged with what we can detect
/// live from disk (existence, authentication, email, active flag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    /// Absolute, tilde-expanded path.
    pub path: PathBuf,
    /// Portable, `~`-relative path as stored in metadata.
    pub raw_path: String,
    pub last_used: Option<DateTime<Utc>>,
    /// The account signed into the directory right now (live from `.claude.json`),
    /// falling back to the bound `expected_email` when nothing is detected.
    pub email: Option<String>,
    /// The account this profile is *bound* to — its stored identity, set once
    /// (at add/adopt/first sign-in) and never silently rebound. Diverges from
    /// `email` only when the directory has been signed into the wrong account.
    pub expected_email: Option<String>,
    /// Brief plan label, e.g. "Pro", "Max 5x", "Team".
    pub plan: Option<String>,
    pub exists: bool,
    pub authenticated: bool,
    pub active: bool,
}

impl Profile {
    /// The parenthetical identity shown after the name: "email · Plan".
    pub fn identity(&self) -> Option<String> {
        match (&self.email, &self.plan) {
            (Some(e), Some(p)) => Some(format!("{e} · {p}")),
            (Some(e), None) => Some(e.clone()),
            (None, Some(p)) => Some(p.clone()),
            (None, None) => None,
        }
    }

    /// Detects a wrong login: the directory is signed into a *different* account
    /// than the one this profile is bound to. Returns `(signed_in, expected)` —
    /// e.g. signing into the `takeyoung` profile as `paul@nhost.io` yields
    /// `("paul@nhost.io", "takeyoung@gmail.com")`. `None` when they match, or when
    /// the profile isn't bound yet, or nothing is signed in.
    pub fn email_mismatch(&self) -> Option<(&str, &str)> {
        match (self.email.as_deref(), self.expected_email.as_deref()) {
            (Some(cur), Some(exp)) if cur != exp => Some((cur, exp)),
            _ => None,
        }
    }
}

/// Validate a profile name. Names become part of a directory name and a
/// symlink comparison, so we keep them to a safe, portable character set.
pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name != crate::paths::reserved_name(); // would collide with the active symlink
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reasonable_names() {
        for name in ["work", "personal", "client-1", "acme_inc", "a"] {
            assert!(validate_name(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn rejects_bad_names() {
        for name in ["", "has space", "with/slash", "dot.name", crate::paths::reserved_name(), &"x".repeat(65)] {
            assert!(validate_name(name).is_err(), "{name:?} should be invalid");
        }
    }
}
