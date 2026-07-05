//! Orchestration layer: the single place that mutates profiles and keeps the
//! symlink and metadata in agreement.
//!
//! Invariants:
//!   * The `~/.claude-switcher` symlink is the source of truth for activation.
//!   * `profiles.json` is a cache for the UI and is reconciled on every load.
//!   * No profile data is ever copied — directories are created in place and
//!     moves use `rename(2)`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::detect;
use crate::error::{Error, Result};
use crate::metadata::{Metadata, ProfileMeta, Settings};
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

    /// Persisted user settings (auto-refresh, poll interval).
    pub fn settings(&self) -> &Settings {
        &self.meta.settings
    }

    /// Persist the auto-refresh preference.
    pub fn set_auto_refresh(&mut self, on: bool) -> Result<()> {
        self.meta.settings.auto_refresh = on;
        self.save()
    }

    /// Persist the compact (minimal) view preference.
    pub fn set_compact(&mut self, on: bool) -> Result<()> {
        self.meta.settings.compact = on;
        self.save()
    }

    /// The last persisted usage snapshot, if any.
    pub fn usage_cache(&self) -> Option<&crate::usage::UsageCache> {
        self.meta.usage_cache.as_ref()
    }

    /// Persist a fresh usage snapshot for reuse by later sessions.
    pub fn save_usage_cache(&mut self, cache: crate::usage::UsageCache) -> Result<()> {
        self.meta.usage_cache = Some(cache);
        self.save()
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
    ///
    /// Ordered for display: the active profile first, then the rest by most
    /// recently used, with never-used profiles at the bottom.
    pub fn profiles(&self) -> Vec<Profile> {
        let active = self.meta.active.as_deref();
        let mut profiles: Vec<Profile> = self
            .meta
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
                let last_used = if exists {
                    detect::last_used(&path, &self.paths.home)
                } else {
                    None
                };
                Profile {
                    name: m.name.clone(),
                    email: account.email.clone().or_else(|| m.email.clone()),
                    plan: account.plan.clone(),
                    raw_path: m.path.clone(),
                    last_used,
                    exists,
                    authenticated: account.authenticated,
                    active: active == Some(m.name.as_str()),
                    path,
                }
            })
            .collect();
        profiles.sort_by(|a, b| {
            // Active always first.
            b.active.cmp(&a.active).then_with(|| {
                // Then most-recently-used first; never-used (None) last.
                match (a.last_used, b.last_used) {
                    (Some(x), Some(y)) => y.cmp(&x),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            })
        });
        profiles
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

    /// Adopt an *existing* Claude configuration directory as a managed profile,
    /// in place and without copying anything. The first profile adopted (or one
    /// adopted with `activate`) becomes active.
    pub fn adopt(&mut self, name: &str, path: &Path, activate: bool) -> Result<Profile> {
        validate_name(name)?;
        if self.meta.contains(name) {
            return Err(Error::DuplicateProfile(name.to_string()));
        }
        if !path.is_dir() {
            return Err(Error::NotAProfileDir(self.paths.contract(path)));
        }
        let canon = canonical(path);
        if self
            .meta
            .profiles
            .iter()
            .any(|m| canonical(&self.paths.expand(&m.path)) == canon)
        {
            return Err(Error::PathAlreadyManaged(self.paths.contract(path)));
        }

        self.meta.profiles.push(ProfileMeta {
            name: name.to_string(),
            path: self.paths.contract(path),
            email: None,
        });
        self.refresh_email(name);

        let first = self.meta.profiles.len() == 1;
        self.save()?;
        if activate || first {
            self.switch(name)?;
        }
        Ok(self
            .profiles()
            .into_iter()
            .find(|p| p.name == name)
            .expect("just adopted"))
    }

    /// First-run convenience: if nothing is managed yet, adopt every Claude
    /// config directory we can discover so the tool immediately reflects the
    /// account(s) you're already signed in to. Returns the names adopted.
    pub fn bootstrap_if_empty(&mut self) -> Result<Vec<String>> {
        if !self.meta.profiles.is_empty() {
            return Ok(Vec::new());
        }
        let mut adopted = Vec::new();
        for (name, path) in self.discover_candidates() {
            if self.adopt(&name, &path, false).is_ok() {
                adopted.push(name);
            }
        }
        Ok(adopted)
    }

    /// Scan the home directory for un-managed Claude config directories
    /// (`~/.claude` and `~/.claude-*`, excluding the active symlink). Returns
    /// suggested `(name, path)` pairs, skipping anything already managed.
    pub fn discover_candidates(&self) -> Vec<(String, PathBuf)> {
        let managed: Vec<PathBuf> = self
            .meta
            .profiles
            .iter()
            .map(|m| canonical(&self.paths.expand(&m.path)))
            .collect();
        let taken: Vec<String> = self.meta.profiles.iter().map(|p| p.name.clone()).collect();

        let Ok(entries) = fs::read_dir(&self.paths.home) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy().into_owned();
            let is_candidate =
                file_name == ".claude" || file_name.starts_with(crate::paths::PROFILE_PREFIX);
            if !is_candidate || file_name == crate::paths::ACTIVE_LINK {
                continue;
            }
            let path = entry.path();
            // Skip the active symlink and any non-directory.
            if path.is_symlink() || !path.is_dir() {
                continue;
            }
            if managed.contains(&canonical(&path)) {
                continue;
            }
            let name = derive_name(&file_name);
            if validate_name(&name).is_err() || taken.contains(&name) {
                continue;
            }
            out.push((name, path));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Import login/onboarding state for the special case of the default
    /// `~/.claude` profile, whose `.claude.json` lives at `~/.claude.json`
    /// rather than inside the directory. Copies (never moves) so the un-wrapped
    /// `claude` keeps working. Returns whether anything was imported.
    pub fn migrate_home_state(&self, name: &str) -> Result<bool> {
        let meta = self
            .meta
            .find(name)
            .ok_or_else(|| Error::UnknownProfile(name.to_string()))?;
        let dir = self.paths.expand(&meta.path);
        let inside = dir.join(".claude.json");
        let home_state = self.paths.home.join(".claude.json");
        if inside.exists() || !home_state.is_file() {
            return Ok(false);
        }
        let bytes = fs::read(&home_state)?;
        symlink::atomic_write(&inside, &bytes)?;
        Ok(true)
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

/// Suggest a profile name from a config directory's file name:
/// `.claude` -> `default`, `.claude-work` -> `work`.
fn derive_name(file_name: &str) -> String {
    if file_name == ".claude" {
        "default".to_string()
    } else if let Some(rest) = file_name.strip_prefix(".claude-") {
        rest.to_string()
    } else {
        file_name.trim_start_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_names() {
        assert_eq!(derive_name(".claude"), "default");
        assert_eq!(derive_name(".claude-work"), "work");
        assert_eq!(derive_name(".claude-takeyoung"), "takeyoung");
    }
}
