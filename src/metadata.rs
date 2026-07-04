//! Persistence of the UI-only metadata file (`profiles.json`).
//!
//! The symlink is the *source of truth* for which profile is active — this file
//! only records display information (order, cached email, last-used time) so the
//! TUI has something to show without probing the filesystem every frame.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::symlink;

/// Metadata for a single profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    /// Stored in portable `~`-relative form when possible.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "lastUsed")]
    pub last_used: Option<DateTime<Utc>>,
    /// Cached email, refreshed from `.claude.json` whenever we detect it live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Top-level document written to disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Cached active-profile name. The symlink remains authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub profiles: Vec<ProfileMeta>,
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
                last_used: Some(Utc::now()),
                email: Some("paul@nhost.io".into()),
            }],
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
