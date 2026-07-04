//! TUI state machine, kept free of any rendering or terminal I/O so it can be
//! unit-tested in isolation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use chrono::{DateTime, Local};

use crate::manager::Manager;
use crate::profile::Profile;
use crate::usage::Usage;

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
    /// When usage was last (re)fetched — drives the header timestamp and the
    /// once-per-minute refresh debounce.
    last_updated: DateTime<Local>,
}

/// Minimum gap between manual usage refreshes.
const REFRESH_COOLDOWN_SECS: i64 = 60;

impl<'m> App<'m> {
    pub fn new(manager: &'m mut Manager) -> Self {
        let profiles = manager.profiles();
        let order = profiles.iter().map(|p| p.name.clone()).collect();
        let home = manager.paths().home.clone();
        let active_link = manager.paths().active_link();
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
        };
        let profiles = app.profiles.clone();
        for p in &profiles {
            app.begin_usage_fetch(p);
        }
        app
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

    /// Drain completed usage lookups into state. Call once per UI tick.
    pub fn pump_usage(&mut self) {
        while let Ok((name, state)) = self.usage_rx.try_recv() {
            self.usage.insert(name, state);
        }
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

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// Whether the header Refresh control is currently focused.
    pub fn refresh_focused(&self) -> bool {
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

    /// Enter: refresh usage if the header control is focused, else switch.
    pub fn activate(&mut self) {
        if self.refresh_focused() {
            self.refresh();
        } else {
            self.switch_selected();
        }
    }

    /// Re-fetch usage for every profile, debounced to once per minute.
    pub fn refresh(&mut self) {
        let elapsed = Local::now().signed_duration_since(self.last_updated);
        let remaining = REFRESH_COOLDOWN_SECS - elapsed.num_seconds();
        if remaining > 0 {
            self.status = Some(format!(
                "Usage refreshes at most once/min — try again in {remaining}s"
            ));
            return;
        }
        self.last_updated = Local::now();
        self.usage.clear();
        let profiles = self.profiles.clone();
        for p in &profiles {
            self.begin_usage_fetch(p);
        }
        self.status = Some("Refreshing usage…".to_string());
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
    fn refresh_row_and_debounce() {
        let dir = tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let mut app = App::new(&mut mgr);
        app.begin_add();
        for ch in "solo".chars() {
            app.input_push(ch);
        }
        app.commit_input();

        // Focus starts on the Refresh control; no profile is selected there.
        app.selected = 0;
        assert!(app.refresh_focused());
        assert!(app.selected_profile().is_none());

        // Enter on the header refreshes, but the just-loaded data is debounced.
        app.activate();
        assert!(
            app.status.as_deref().unwrap().contains("once/min"),
            "got {:?}",
            app.status
        );

        // Navigation wraps over [refresh, solo].
        app.select_next();
        assert_eq!(app.selected_profile().unwrap().name, "solo");
        app.select_next();
        assert!(app.refresh_focused());
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
