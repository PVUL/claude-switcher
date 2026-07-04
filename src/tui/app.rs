//! TUI state machine, kept free of any rendering or terminal I/O so it can be
//! unit-tested in isolation.

use crate::manager::Manager;
use crate::profile::Profile;

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
    pub selected: usize,
    pub mode: Mode,
    pub status: Option<String>,
    pub should_quit: bool,
}

impl<'m> App<'m> {
    pub fn new(manager: &'m mut Manager) -> Self {
        let profiles = manager.profiles();
        let order = profiles.iter().map(|p| p.name.clone()).collect();
        App {
            manager,
            profiles,
            order,
            selected: 0,
            mode: Mode::Normal,
            status: None,
            should_quit: false,
        }
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn selected_profile(&self) -> Option<&Profile> {
        self.profiles.get(self.selected)
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
        if self.selected >= self.profiles.len() {
            self.selected = self.profiles.len().saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if !self.profiles.is_empty() {
            self.selected = (self.selected + 1) % self.profiles.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.profiles.is_empty() {
            self.selected = (self.selected + self.profiles.len() - 1) % self.profiles.len();
        }
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
            self.selected = idx;
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

        // Navigate and switch.
        app.selected = app.profiles().iter().position(|p| p.name == "personal").unwrap();
        app.switch_selected();
        assert!(app.profiles().iter().find(|p| p.name == "personal").unwrap().active);

        // Rename selected.
        app.begin_rename();
        app.input_backspace();
        app.input_push('X');
        app.commit_input(); // personal -> personaX
        assert!(app.profiles().iter().any(|p| p.name == "personaX"));

        // Delete (kept dir).
        app.selected = app.profiles().iter().position(|p| p.name == "work").unwrap();
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
        app.selected = app.profiles().iter().position(|p| p.name == "c").unwrap();
        app.switch_selected();
        let after: Vec<String> = app.profiles().iter().map(|p| p.name.clone()).collect();
        assert_eq!(before, after, "order changed after switch");
        // The highlight follows the profile we switched to.
        assert_eq!(app.selected_profile().unwrap().name, "c");
        assert!(app.selected_profile().unwrap().active);
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
