//! Orchestration layer: the single place that mutates profiles and keeps the
//! symlink and metadata in agreement.
//!
//! Invariants:
//!   * The `~/.claude-active` symlink is the source of truth for activation.
//!   * `profiles.json` is a cache for the UI and is reconciled on every load.
//!   * No profile data is ever copied — directories are created in place and
//!     moves use `rename(2)`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::detect;
use crate::error::{Error, Result};
use crate::metadata::{Metadata, ProfileMeta};
use crate::paths::Paths;
use crate::profile::{validate_name, Profile};
use crate::symlink;

pub struct Manager {
    paths: Paths,
    meta: Metadata,
}

impl Manager {
    /// Load state from disk and reconcile the cached active name with the
    /// symlink (which always wins).
    pub fn load(paths: Paths) -> Result<Self> {
        let mut meta = Metadata::load(&paths.metadata_file())?;
        let active = Self::active_name_from_link(&paths, &meta);
        if meta.active != active {
            meta.active = active;
        }
        Ok(Self { paths, meta })
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// Determine the active profile name by resolving the symlink target and
    /// matching it against known profile paths.
    fn active_name_from_link(paths: &Paths, meta: &Metadata) -> Option<String> {
        let target = symlink::read_link(&paths.active_link())?;
        let target = canonical(&target);
        meta.profiles
            .iter()
            .find(|p| canonical(&paths.expand(&p.path)) == target)
            .map(|p| p.name.clone())
    }

    /// Build the enriched, display-ready list of profiles.
    pub fn profiles(&self) -> Vec<Profile> {
        let active = self.meta.active.as_deref();
        self.meta
            .profiles
            .iter()
            .map(|m| {
                let path = self.paths.expand(&m.path);
                let exists = path.is_dir();
                let account = if exists {
                    detect::inspect(&path, &self.paths.home)
                } else {
                    detect::Account::default()
                };
                Profile {
                    name: m.name.clone(),
                    email: account.email.clone().or_else(|| m.email.clone()),
                    raw_path: m.path.clone(),
                    last_used: m.last_used,
                    exists,
                    authenticated: account.authenticated,
                    active: active == Some(m.name.as_str()),
                    path,
                }
            })
            .collect()
    }

    pub fn active(&self) -> Option<Profile> {
        self.profiles().into_iter().find(|p| p.active)
    }

    /// Add a new profile. Creates the directory if it does not exist. The first
    /// profile added automatically becomes active.
    pub fn add(&mut self, name: &str, custom_path: Option<&Path>) -> Result<Profile> {
        validate_name(name)?;
        if self.meta.contains(name) {
            return Err(Error::DuplicateProfile(name.to_string()));
        }
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => self.paths.default_profile_path(name),
        };
        fs::create_dir_all(&path)?;

        self.meta.profiles.push(ProfileMeta {
            name: name.to_string(),
            path: self.paths.contract(&path),
            last_used: None,
            email: None,
        });
        self.refresh_email(name);

        let first = self.meta.profiles.len() == 1;
        self.save()?;
        if first {
            self.switch(name)?;
        }
        Ok(self
            .profiles()
            .into_iter()
            .find(|p| p.name == name)
            .expect("just added"))
    }

    /// Switch the active profile by repointing the symlink atomically.
    pub fn switch(&mut self, name: &str) -> Result<()> {
        let meta = self
            .meta
            .find(name)
            .ok_or_else(|| Error::UnknownProfile(name.to_string()))?;
        let target = self.paths.expand(&meta.path);
        fs::create_dir_all(&target)?;

        symlink::atomic_symlink(&target, &self.paths.active_link())?;

        self.meta.active = Some(name.to_string());
        if let Some(m) = self.meta.find_mut(name) {
            m.last_used = Some(Utc::now());
        }
        self.refresh_email(name);
        self.save()
    }

    /// Rename a profile. If it lives at the default location it is moved to the
    /// new default location (a `rename`, not a copy) and the symlink is
    /// re-pointed if the profile was active.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        validate_name(new)?;
        if !self.meta.contains(old) {
            return Err(Error::UnknownProfile(old.to_string()));
        }
        if old == new {
            return Ok(());
        }
        if self.meta.contains(new) {
            return Err(Error::DuplicateProfile(new.to_string()));
        }

        let was_active = self.meta.active.as_deref() == Some(old);
        let old_path = self.paths.expand(&self.meta.find(old).unwrap().path);
        let at_default = old_path == self.paths.default_profile_path(old);

        let new_path = if at_default {
            let new_path = self.paths.default_profile_path(new);
            if old_path.exists() {
                fs::rename(&old_path, &new_path)?;
            }
            new_path
        } else {
            old_path
        };

        let m = self.meta.find_mut(old).unwrap();
        m.name = new.to_string();
        m.path = self.paths.contract(&new_path);

        if was_active {
            self.meta.active = Some(new.to_string());
            symlink::atomic_symlink(&new_path, &self.paths.active_link())?;
        }
        self.save()
    }

    /// Remove a profile from management. The directory is left in place unless
    /// `purge` is set. If the removed profile was active, activation moves to
    /// another profile when one exists.
    pub fn remove(&mut self, name: &str, purge: bool) -> Result<()> {
        let meta = self
            .meta
            .find(name)
            .ok_or_else(|| Error::UnknownProfile(name.to_string()))?
            .clone();
        let path = self.paths.expand(&meta.path);
        let was_active = self.meta.active.as_deref() == Some(name);

        if was_active && self.meta.profiles.len() == 1 {
            return Err(Error::CannotDeleteLastActive);
        }
        if purge && path.is_dir() {
            fs::remove_dir_all(&path)?;
        }

        self.meta.profiles.retain(|p| p.name != name);

        if was_active {
            // Fall back to the first remaining profile.
            if let Some(next) = self.meta.profiles.first().map(|p| p.name.clone()) {
                self.switch(&next)?;
            } else {
                self.meta.active = None;
                let _ = fs::remove_file(self.paths.active_link());
            }
        }
        self.save()
    }

    /// Refresh the cached email for a profile from its `.claude.json`.
    fn refresh_email(&mut self, name: &str) {
        let (path, home) = match self.meta.find(name) {
            Some(m) => (self.paths.expand(&m.path), self.paths.home.clone()),
            None => return,
        };
        if let Some(email) = detect::inspect(&path, &home).email {
            if let Some(m) = self.meta.find_mut(name) {
                m.email = Some(email);
            }
        }
    }

    fn save(&self) -> Result<()> {
        self.meta.save(&self.paths.metadata_file())
    }
}

/// Best-effort canonicalization: fall back to the raw path if the target does
/// not exist yet (e.g. a symlink pointing at a not-yet-created directory).
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
