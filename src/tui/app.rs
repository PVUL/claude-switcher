//! TUI state machine, kept free of any rendering or terminal I/O so it can be
//! unit-tested in isolation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use chrono::{DateTime, Local, Utc};

use crate::manager::Manager;
use crate::profile::Profile;
use crate::usage::{Usage, UsageCache};

/// Per-profile usage-fetch state, updated as background lookups complete.
#[derive(Debug, Clone)]
pub enum UsageState {
    Loading,
    Ready(Usage),
    Unavailable,
}

/// What the user is currently doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Text entry for adding or renaming.
    Input { action: InputAction, buffer: String },
    /// Awaiting y/n confirmation for a delete.
    ConfirmDelete { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    Add,
    Rename { from: String },
}

pub struct App<'m> {
    manager: &'m mut Manager,
    profiles: Vec<Profile>,
    /// Display order (profile names), fixed for the session so switching does
    /// not shuffle the list. New profiles append; removed ones drop out.
    order: Vec<String>,
    /// Selected row. Row 0 is the header "Refresh" control; profile `i` is at
    /// row `i + 1`.
    pub selected: usize,
    pub mode: Mode,
    pub status: Option<String>,
    pub should_quit: bool,
    // Background usage lookups.
    home: PathBuf,
    active_link: PathBuf,
    usage: HashMap<String, UsageState>,
    usage_tx: Sender<(String, UsageState)>,
    usage_rx: Receiver<(String, UsageState)>,
    /// When usage was last (re)fetched — drives the header timestamp, the
    /// once-per-minute manual-refresh debounce, and the auto-refresh timer.
    last_updated: DateTime<Local>,
    /// Whether usage is polled on a timer (persisted).
    auto_refresh: bool,
    /// Auto-refresh interval, in seconds (from config).
    poll_interval_secs: u64,
    /// Set when a fetch batch is in progress; drives persisting the snapshot
    /// once every lookup has resolved.
    usage_persist_pending: bool,
}

/// Minimum gap between manual usage refreshes.
const REFRESH_COOLDOWN_SECS: i64 = 60;

impl<'m> App<'m> {
    pub fn new(manager: &'m mut Manager) -> Self {
        let profiles = manager.profiles();
        let order = profiles.iter().map(|p| p.name.clone()).collect();
        let home = manager.paths().home.clone();
        let active_link = manager.paths().active_link();
        let auto_refresh = manager.settings().auto_refresh;
        let poll_interval_secs = manager.settings().poll_interval_secs;
        let (usage_tx, usage_rx) = mpsc::channel();
        let mut app = App {
            manager,
            profiles,
            order,
            // Start focused on the Refresh control so every profile row is
            // shown unencumbered by selection styling.
            selected: 0,
            mode: Mode::Normal,
            status: None,
            should_quit: false,
            home,
            active_link,
            usage: HashMap::new(),
            usage_tx,
            usage_rx,
            last_updated: Local::now(),
            auto_refresh,
            poll_interval_secs,
            usage_persist_pending: false,
        };
        app.seed_usage();
        app
    }

    /// Reuse the persisted usage snapshot if it's still within the poll window;
    /// otherwise fetch fresh. When reused, `last_updated` is set to the cached
    /// fetch time so the next auto-refresh lands exactly at the interval mark
    /// (e.g. a 9-minute-old snapshot refreshes in 1 minute for a 10-min poll).
    fn seed_usage(&mut self) {
        let fresh = self.manager.usage_cache().and_then(|c| {
            let age = Utc::now().signed_duration_since(c.fetched_at).num_seconds();
            (age >= 0 && age < self.poll_interval_secs as i64).then(|| c.clone())
        });
        match fresh {
            Some(cache) => {
                self.last_updated = cache.fetched_at.with_timezone(&Local);
                let names: Vec<String> = self.profiles.iter().map(|p| p.name.clone()).collect();
                for name in names {
                    let state = match cache.profiles.get(&name) {
                        Some(u) => UsageState::Ready(u.clone()),
                        None => UsageState::Unavailable,
                    };
                    self.usage.insert(name, state);
                }
            }
            None => {
                let profiles = self.profiles.clone();
                for p in &profiles {
                    self.begin_usage_fetch(p);
                }
                self.usage_persist_pending = true;
            }
        }
    }

    /// Kick off a background usage lookup for a profile, if not already tracked.
    fn begin_usage_fetch(&mut self, profile: &Profile) {
        if self.usage.contains_key(&profile.name) {
            return;
        }
        if !profile.exists {
            self.usage.insert(profile.name.clone(), UsageState::Unavailable);
            return;
        }
        self.usage.insert(profile.name.clone(), UsageState::Loading);
        let tx = self.usage_tx.clone();
        let name = profile.name.clone();
        let path = profile.path.clone();
        let home = self.home.clone();
        let link = if profile.active {
            Some(self.active_link.clone())
        } else {
            None
        };
        thread::spawn(move || {
            let state = match crate::usage::fetch(&path, &home, link.as_deref()) {
                Some(u) => UsageState::Ready(u),
                None => UsageState::Unavailable,
            };
            let _ = tx.send((name, state));
        });
    }

    /// Drain completed usage lookups into state. Call once per UI tick. Once a
    /// fetch batch fully resolves, persist the snapshot for later sessions.
    pub fn pump_usage(&mut self) {
        let mut changed = false;
        while let Ok((name, state)) = self.usage_rx.try_recv() {
            self.usage.insert(name, state);
            changed = true;
        }
        if changed && self.usage_persist_pending && !self.is_refreshing() {
            self.persist_usage_cache();
            self.usage_persist_pending = false;
        }
    }

    fn persist_usage_cache(&mut self) {
        let profiles = self
            .usage
            .iter()
            .filter_map(|(name, st)| match st {
                UsageState::Ready(u) => Some((name.clone(), u.clone())),
                _ => None,
            })
            .collect();
        let cache = UsageCache {
            fetched_at: self.last_updated.with_timezone(&Utc),
            profiles,
        };
        let _ = self.manager.save_usage_cache(cache);
    }

    pub fn usage(&self, name: &str) -> Option<&UsageState> {
        self.usage.get(name)
    }

    /// True while any usage lookup is still in flight.
    pub fn is_refreshing(&self) -> bool {
        self.usage.values().any(|s| matches!(s, UsageState::Loading))
    }

    /// Header label: "updating…" while fetching, else "updated 3:49pm".
    pub fn updated_label(&self) -> String {
        if self.is_refreshing() {
            "updating…".to_string()
        } else {
            format!("updated {}", self.last_updated.format("%-I:%M%P"))
        }
    }

    /// Header toggle label, e.g. "auto-refresh: on".
    pub fn auto_refresh_label(&self) -> String {
        format!("auto-refresh: {}", if self.auto_refresh { "on" } else { "off" })
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// Whether the header control (the auto-refresh toggle) is focused.
    pub fn header_focused(&self) -> bool {
        self.selected == 0
    }

    /// Index into `profiles()` of the selected row, or `None` when the Refresh
    /// control is focused.
    pub fn selected_profile_index(&self) -> Option<usize> {
        self.selected.checked_sub(1)
    }

    pub fn selected_profile(&self) -> Option<&Profile> {
        self.selected_profile_index().and_then(|i| self.profiles.get(i))
    }

    /// Total selectable rows: the Refresh control plus every profile.
    fn row_count(&self) -> usize {
        self.profiles.len() + 1
    }

    fn reload(&mut self) {
        // Re-fetch live state but preserve the session's display order so the
        // list doesn't reshuffle when the active profile changes.
        let fresh = self.manager.profiles();
        let mut ordered: Vec<Profile> = Vec::with_capacity(fresh.len());
        for name in &self.order {
            if let Some(p) = fresh.iter().find(|p| &p.name == name) {
                ordered.push(p.clone());
            }
        }
        // Append profiles added since the session started (in manager order).
        for p in &fresh {
            if !self.order.contains(&p.name) {
                ordered.push(p.clone());
            }
        }
        self.order = ordered.iter().map(|p| p.name.clone()).collect();
        self.profiles = ordered;
        // Rows are 0..=profiles.len() (row 0 is Refresh); clamp if a profile
        // was removed.
        if self.selected > self.profiles.len() {
            self.selected = self.profiles.len();
        }
        // Fetch usage for any profile added since the session started.
        let profiles = self.profiles.clone();
        for p in &profiles {
            self.begin_usage_fetch(p);
        }
    }

    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1) % self.row_count();
    }

    pub fn select_prev(&mut self) {
        let n = self.row_count();
        self.selected = (self.selected + n - 1) % n;
    }

    /// Enter: toggle auto-refresh if the header control is focused, else switch.
    pub fn activate(&mut self) {
        if self.header_focused() {
            self.toggle_auto_refresh();
        } else {
            self.switch_selected();
        }
    }

    /// Toggle and persist the auto-refresh preference.
    pub fn toggle_auto_refresh(&mut self) {
        self.auto_refresh = !self.auto_refresh;
        let _ = self.manager.set_auto_refresh(self.auto_refresh);
        self.status = Some(format!(
            "Auto-refresh {}",
            if self.auto_refresh { "on" } else { "off" }
        ));
    }

    /// Manual refresh ('r'): re-fetch usage, debounced to once per minute. A
    /// successful refresh also resets the auto-refresh timer.
    pub fn manual_refresh(&mut self) {
        let remaining =
            REFRESH_COOLDOWN_SECS - Local::now().signed_duration_since(self.last_updated).num_seconds();
        if remaining > 0 {
            self.status = Some(format!(
                "Usage refreshes at most once/min — try again in {remaining}s"
            ));
            return;
        }
        // The header shows "updating…" → "updated <time>", so no sticky footer
        // message is needed (and a sticky one would linger after completion).
        self.status = None;
        self.do_refresh();
    }

    /// Called each UI tick: re-fetch when auto-refresh is on and the interval
    /// has elapsed. (The interval is well above the manual debounce window.)
    pub fn tick_auto_refresh(&mut self) {
        if !self.auto_refresh {
            return;
        }
        let elapsed = Local::now().signed_duration_since(self.last_updated).num_seconds();
        if elapsed >= self.poll_interval_secs as i64 {
            self.do_refresh();
        }
    }

    /// Reset usage to Loading and spawn fresh lookups; resets the poll timer
    /// and marks the snapshot for re-persisting once complete.
    fn do_refresh(&mut self) {
        self.last_updated = Local::now();
        self.usage.clear();
        let profiles = self.profiles.clone();
        for p in &profiles {
            self.begin_usage_fetch(p);
        }
        self.usage_persist_pending = true;
    }

    pub fn switch_selected(&mut self) {
        let Some(name) = self.selected_profile().map(|p| p.name.clone()) else {
            return;
        };
        match self.manager.switch(&name) {
            Ok(()) => self.status = Some(format!("Switched to '{name}'.")),
            Err(e) => self.status = Some(format!("Error: {e}")),
        }
        self.reload();
        // The list reorders (active moves to the top); keep the highlight on
        // the profile we just switched to rather than a fixed row index.
        self.select_by_name(&name);
    }

    fn select_by_name(&mut self, name: &str) {
        if let Some(idx) = self.profiles.iter().position(|p| p.name == name) {
            self.selected = idx + 1;
        }
    }

    pub fn begin_add(&mut self) {
        self.mode = Mode::Input {
            action: InputAction::Add,
            buffer: String::new(),
        };
    }

    pub fn begin_rename(&mut self) {
        if let Some(p) = self.selected_profile() {
            self.mode = Mode::Input {
                action: InputAction::Rename { from: p.name.clone() },
                buffer: p.name.clone(),
            };
        }
    }

    pub fn begin_delete(&mut self) {
        if let Some(p) = self.selected_profile() {
            self.mode = Mode::ConfirmDelete { name: p.name.clone() };
        }
    }

    pub fn cancel(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn input_push(&mut self, c: char) {
        if let Mode::Input { buffer, .. } = &mut self.mode {
            buffer.push(c);
        }
    }

    pub fn input_backspace(&mut self) {
        if let Mode::Input { buffer, .. } = &mut self.mode {
            buffer.pop();
        }
    }

    /// Commit the active input (Enter pressed).
    pub fn commit_input(&mut self) {
        let Mode::Input { action, buffer } = self.mode.clone() else {
            return;
        };
        let name = buffer.trim().to_string();
        let result = match action {
            InputAction::Add => self.manager.add(&name, None).map(|_| format!("Added '{name}'.")),
            InputAction::Rename { from } => self
                .manager
                .rename(&from, &name)
                .map(|_| format!("Renamed to '{name}'.")),
        };
        match result {
            Ok(msg) => {
                self.status = Some(msg);
                self.mode = Mode::Normal;
                self.reload();
                self.select_by_name(&name);
            }
            // Keep the input open so the user can fix the name.
            Err(e) => self.status = Some(format!("Error: {e}")),
        }
    }

    /// Confirm a pending delete (does not purge the directory).
    pub fn confirm_delete(&mut self) {
        let Mode::ConfirmDelete { name } = self.mode.clone() else {
            return;
        };
        match self.manager.remove(&name, false) {
            Ok(()) => self.status = Some(format!("Removed '{name}' (directory kept).")),
            Err(e) => self.status = Some(format!("Error: {e}")),
        }
        self.mode = Mode::Normal;
        self.reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use tempfile::tempdir;

    fn manager(dir: &std::path::Path) -> Manager {
        let paths = Paths::with_roots(dir.join("home"), dir.join("cfg"));
        std::fs::create_dir_all(&paths.home).unwrap();
        Manager::load(paths).unwrap()
    }

    #[test]
    fn add_switch_rename_delete_flow() {
        let dir = tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let mut app = App::new(&mut mgr);

        // Add via input.
        app.begin_add();
        for c in "work".chars() {
            app.input_push(c);
        }
        app.commit_input();
        assert_eq!(app.profiles().len(), 1);
        assert!(app.profiles()[0].active, "first profile auto-activates");

        // Add a second one.
        app.begin_add();
        for c in "personal".chars() {
            app.input_push(c);
        }
        app.commit_input();
        assert_eq!(app.profiles().len(), 2);

        // Navigate and switch (row 0 is Refresh, so profiles start at row 1).
        app.selected = app.profiles().iter().position(|p| p.name == "personal").unwrap() + 1;
        app.switch_selected();
        assert!(app.profiles().iter().find(|p| p.name == "personal").unwrap().active);

        // Rename selected.
        app.begin_rename();
        app.input_backspace();
        app.input_push('X');
        app.commit_input(); // personal -> personaX
        assert!(app.profiles().iter().any(|p| p.name == "personaX"));

        // Delete (kept dir).
        app.selected = app.profiles().iter().position(|p| p.name == "work").unwrap() + 1;
        app.begin_delete();
        app.confirm_delete();
        assert_eq!(app.profiles().len(), 1);
    }

    #[test]
    fn order_is_stable_across_switch() {
        let dir = tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let mut app = App::new(&mut mgr);
        for name in ["a", "b", "c"] {
            app.begin_add();
            for ch in name.chars() {
                app.input_push(ch);
            }
            app.commit_input();
        }
        let before: Vec<String> = app.profiles().iter().map(|p| p.name.clone()).collect();

        // Switching must not reshuffle the rows within the session.
        app.selected = app.profiles().iter().position(|p| p.name == "c").unwrap() + 1;
        app.switch_selected();
        let after: Vec<String> = app.profiles().iter().map(|p| p.name.clone()).collect();
        assert_eq!(before, after, "order changed after switch");
        // The highlight follows the profile we switched to.
        assert_eq!(app.selected_profile().unwrap().name, "c");
        assert!(app.selected_profile().unwrap().active);
    }

    #[test]
    fn header_toggles_auto_refresh_and_persists() {
        let dir = tempdir().unwrap();
        {
            let mut mgr = manager(dir.path());
            let mut app = App::new(&mut mgr);
            app.begin_add();
            for ch in "solo".chars() {
                app.input_push(ch);
            }
            app.commit_input();

            // Focus starts on the header toggle; no profile is selected there.
            app.selected = 0;
            assert!(app.header_focused());
            assert!(app.selected_profile().is_none());
            assert_eq!(app.auto_refresh_label(), "auto-refresh: off");

            // Enter toggles auto-refresh on.
            app.activate();
            assert_eq!(app.auto_refresh_label(), "auto-refresh: on");
        }
        // Persisted across a reload.
        let mut mgr = manager(dir.path());
        assert!(mgr.settings().auto_refresh);
        let app = App::new(&mut mgr);
        assert_eq!(app.auto_refresh_label(), "auto-refresh: on");
    }

    #[test]
    fn reuses_fresh_usage_cache() {
        use crate::usage::Window;
        let dir = tempdir().unwrap();
        let mut mgr = manager(dir.path());
        mgr.add("work", None).unwrap();
        // A snapshot from 2 minutes ago (well within the default 10-min window).
        let cache = UsageCache {
            fetched_at: Utc::now() - chrono::Duration::seconds(120),
            profiles: HashMap::from([(
                "work".to_string(),
                Usage {
                    five_hour: Some(Window { utilization: 42.0, resets_at: None }),
                    seven_day: None,
                    seven_day_opus: None,
                },
            )]),
        };
        mgr.save_usage_cache(cache).unwrap();

        // Opening reuses the cache instead of fetching.
        let app = App::new(&mut mgr);
        match app.usage("work") {
            Some(UsageState::Ready(u)) => {
                assert_eq!(u.five_hour.as_ref().unwrap().utilization, 42.0)
            }
            other => panic!("expected cached Ready, got {other:?}"),
        }
    }

    #[test]
    fn manual_refresh_is_debounced() {
        let dir = tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let mut app = App::new(&mut mgr);
        app.begin_add();
        for ch in "solo".chars() {
            app.input_push(ch);
        }
        app.commit_input();

        // Just-loaded data is fresh, so a manual refresh is rate-limited.
        app.manual_refresh();
        assert!(
            app.status.as_deref().unwrap().contains("once/min"),
            "got {:?}",
            app.status
        );

        // Navigation wraps over [header, solo].
        app.selected = 0;
        assert!(app.header_focused());
        app.select_next();
        assert_eq!(app.selected_profile().unwrap().name, "solo");
        app.select_next();
        assert!(app.header_focused());
    }

    #[test]
    fn invalid_name_keeps_input_open() {
        let dir = tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let mut app = App::new(&mut mgr);
        app.begin_add();
        app.input_push('b');
        app.input_push('a');
        app.input_push('d');
        app.input_push('/');
        app.commit_input();
        assert!(matches!(app.mode, Mode::Input { .. }), "stays in input on error");
        assert!(app.status.as_deref().unwrap().starts_with("Error"));
    }
}
