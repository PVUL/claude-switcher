//! Filesystem locations used by claude-switcher.
//!
//! Everything is derived from two roots:
//!   * `home`       — where the profile directories and the active symlink live.
//!   * `config_dir` — where the metadata file (`profiles.json`) lives.
//!
//! Both can be overridden with environment variables, which keeps the whole
//! program testable without touching the real `$HOME`:
//!   * `CLAUDE_SWITCHER_HOME`       overrides the home root.
//!   * `CLAUDE_SWITCHER_CONFIG_DIR` overrides the config directory.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Name of the symlink that every consumer (`claude-switcher-exec`, Pi, wrappers)
/// points `CLAUDE_CONFIG_DIR` at.
pub const ACTIVE_LINK: &str = ".claude-switcher";

/// Prefix for the per-profile Claude configuration directories.
pub const PROFILE_PREFIX: &str = ".claude-";

/// The profile name that would collide with the active symlink (the suffix of
/// [`ACTIVE_LINK`], e.g. `switcher` for `.claude-switcher`) and is therefore
/// reserved.
pub fn reserved_name() -> &'static str {
    ACTIVE_LINK.strip_prefix(PROFILE_PREFIX).unwrap_or("")
}

/// Resolved locations for the current invocation.
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub config_dir: PathBuf,
}

impl Paths {
    /// Discover paths from the environment (or the overrides above).
    pub fn discover() -> Result<Self> {
        let home = match std::env::var_os("CLAUDE_SWITCHER_HOME") {
            Some(h) => PathBuf::from(h),
            None => dirs::home_dir().ok_or(Error::NoHomeDir)?,
        };
        let config_dir = match std::env::var_os("CLAUDE_SWITCHER_CONFIG_DIR") {
            Some(c) => PathBuf::from(c),
            None => home.join(".config").join("claude-switcher"),
        };
        Ok(Self { home, config_dir })
    }

    /// Build paths explicitly (used by tests).
    #[cfg(test)]
    pub fn with_roots(home: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            config_dir: config_dir.into(),
        }
    }

    /// The active-profile symlink, e.g. `~/.claude-switcher`.
    pub fn active_link(&self) -> PathBuf {
        self.home.join(ACTIVE_LINK)
    }

    /// Default directory for a profile of the given name, e.g. `~/.claude-work`.
    pub fn default_profile_path(&self, name: &str) -> PathBuf {
        self.home.join(format!("{PROFILE_PREFIX}{name}"))
    }

    /// The metadata file: `<config_dir>/profiles.json`.
    pub fn metadata_file(&self) -> PathBuf {
        self.config_dir.join("profiles.json")
    }

    /// Expand a stored path that may begin with `~`.
    pub fn expand(&self, raw: &str) -> PathBuf {
        if let Some(rest) = raw.strip_prefix("~/") {
            self.home.join(rest)
        } else if raw == "~" {
            self.home.clone()
        } else {
            PathBuf::from(raw)
        }
    }

    /// Contract an absolute path back to `~`-relative form for portable storage.
    pub fn contract(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths::with_roots("/home/alice", "/home/alice/.config/claude-switcher")
    }

    #[test]
    fn expands_tilde() {
        let p = paths();
        assert_eq!(p.expand("~/.claude-work"), PathBuf::from("/home/alice/.claude-work"));
        assert_eq!(p.expand("~"), PathBuf::from("/home/alice"));
        assert_eq!(p.expand("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn contracts_to_tilde() {
        let p = paths();
        assert_eq!(p.contract(Path::new("/home/alice/.claude-work")), "~/.claude-work");
        assert_eq!(p.contract(Path::new("/home/alice")), "~");
        assert_eq!(p.contract(Path::new("/other/place")), "/other/place");
    }

    #[test]
    fn derived_locations() {
        let p = paths();
        assert_eq!(p.active_link(), PathBuf::from("/home/alice/.claude-switcher"));
        assert_eq!(p.default_profile_path("work"), PathBuf::from("/home/alice/.claude-work"));
        assert_eq!(
            p.metadata_file(),
            PathBuf::from("/home/alice/.config/claude-switcher/profiles.json")
        );
    }
}
