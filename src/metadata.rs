//! Persistence of the UI-only metadata file (`profiles.json`).
//!
//! The symlink is the *source of truth* for which profile is active — this file
//! only records display information (order, cached email, last-used time) so the
//! TUI has something to show without probing the filesystem every frame.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::symlink;
use crate::usage::UsageCache;

/// Metadata for a single profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    /// Stored in portable `~`-relative form when possible.
    pub path: String,
    /// Cached email, refreshed from `.claude.json` whenever we detect it live.
    /// (Last-used is derived from filesystem activity, not stored here.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Top-level document written to disk.
/// User-adjustable settings, persisted alongside the profiles so they survive
/// across sessions. The polling interval can be hand-edited in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Whether the TUI polls usage on a timer.
    #[serde(default, rename = "autoRefresh")]
    pub auto_refresh: bool,
    /// Auto-refresh interval in seconds (default 5 minutes).
    #[serde(default = "default_poll_interval", rename = "pollIntervalSecs")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    300
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_refresh: false,
            poll_interval_secs: default_poll_interval(),
        }
    }
}

// Note: no `Eq` — the usage cache carries `f64` utilization values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    /// Cached active-profile name. The symlink remains authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub profiles: Vec<ProfileMeta>,
    #[serde(default)]
    pub settings: Settings,
    /// Last usage snapshot, reused while still within the poll interval.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "usageCache")]
    pub usage_cache: Option<UsageCache>,
}

impl Metadata {
    /// Load metadata, returning an empty document if the file does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) if contents.trim().is_empty() => Ok(Self::default()),
            Ok(contents) => serde_json::from_str(&contents).map_err(|source| Error::Metadata {
                path: path.display().to_string(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist metadata atomically (write to a temp file, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string_pretty(self).expect("metadata is always serializable");
        json.push('\n');
        symlink::atomic_write(path, json.as_bytes())?;
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&ProfileMeta> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut ProfileMeta> {
        self.profiles.iter_mut().find(|p| p.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.profiles.iter().any(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let meta = Metadata::load(&dir.path().join("nope.json")).unwrap();
        assert_eq!(meta, Metadata::default());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let meta = Metadata {
            active: Some("work".into()),
            profiles: vec![ProfileMeta {
                name: "work".into(),
                path: "~/.claude-work".into(),
                email: Some("paul@nhost.io".into()),
            }],
            settings: Settings {
                auto_refresh: true,
                poll_interval_secs: 300,
            },
            usage_cache: None,
        };
        meta.save(&path).unwrap();
        let reloaded = Metadata::load(&path).unwrap();
        assert_eq!(meta, reloaded);
    }

    #[test]
    fn rejects_malformed_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(Metadata::load(&path), Err(Error::Metadata { .. })));
    }
}
