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
    pub email: Option<String>,
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
        for name in [
            "",
            "has space",
            "with/slash",
            "dot.name",
            crate::paths::reserved_name(),
            &"x".repeat(65),
        ] {
            assert!(validate_name(name).is_err(), "{name:?} should be invalid");
        }
    }
}
