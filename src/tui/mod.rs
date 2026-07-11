//! Interactive terminal UI: terminal lifecycle and the input event loop.

mod app;
mod ui;

use std::io::{self, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::error::Result;
use crate::manager::Manager;

use app::{App, Mode};
use ui::{CHROME_LINES, COMPACT_ROWS_PER_PROFILE, ROWS_PER_PROFILE};

/// Entry point from `main` when no subcommand is given.
pub fn run(manager: &mut Manager) -> Result<()> {
    if manager.profiles().is_empty() {
        eprintln!("No profiles yet. Add one with:  claude-switcher add <name>");
        return Ok(());
    }

    let mut app = App::new(manager);
    // The inline viewport's height is fixed at creation, so toggling the view
    // mode (which changes the per-profile height) rebuilds the terminal at the
    // new size while `app` — selection, usage, timers — lives across rebuilds.
    loop {
        let height = viewport_height(app.profiles().len(), app.compact());
        let mut terminal = setup_terminal(height)?;
        let result = event_loop(&mut terminal, &mut app);
        restore_terminal(&mut terminal)?;
        result?;
        if app.should_quit {
            break;
        }
    }
    // Once the TUI is fully torn down (raw mode off, viewport cleared), launch
    // `claude` in a just-added account if the user chose to sign in.
    if let Some(dir) = app.take_launch_login() {
        launch_claude_login(&dir);
    }
    Ok(())
}

/// Run `claude` so the user can sign in to a freshly-added account. Mirrors
/// `claude-switcher-exec`: pins `CLAUDE_CONFIG_DIR` to the resolved profile dir
/// (not the `~/.claude-switcher` symlink) so the OAuth token lands in this
/// account's own slot. `claude` inherits the terminal for the interactive flow.
fn launch_claude_login(config_dir: &std::path::Path) {
    eprintln!("Launching Claude to sign in… (Ctrl-C to cancel)");
    let status = std::process::Command::new("claude")
        .env("CLAUDE_CONFIG_DIR", config_dir)
        .status();
    if let Err(e) = status {
        eprintln!(
            "claude-switcher: could not launch `claude` ({e}). Sign in yourself with:\n  \
             CLAUDE_CONFIG_DIR={} claude",
            config_dir.display()
        );
    }
}

/// Size the inline viewport to the content, capped so it never dominates the
/// terminal. The list scrolls if there are more profiles than fit.
fn viewport_height(profiles: usize, compact: bool) -> u16 {
    let per = if compact { COMPACT_ROWS_PER_PROFILE } else { ROWS_PER_PROFILE };
    let content = CHROME_LINES + per * profiles.max(1) as u16;
    content.clamp(CHROME_LINES + per, 28)
}

fn setup_terminal(height: u16) -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    // Clear the inline viewport so no stale UI is left in the scrollback.
    terminal.clear()?;
    terminal.show_cursor()?;
    Ok(())
}

/// Run the input loop against the current viewport. Returns when the user quits
/// or toggles the view mode; the caller rebuilds the viewport in the latter
/// case (signaled via `App::take_view_dirty`).
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        app.pump_usage();
        app.tick_auto_refresh();
        terminal.draw(|f| ui::draw(f, app))?;

        // Poll so the UI keeps refreshing as background usage lookups land,
        // rather than blocking indefinitely on a keypress.
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        handle_key(app, key);
        // A view toggle needs a differently sized viewport; hand back so the
        // caller can rebuild it.
        if app.should_quit || app.take_view_dirty() {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Ctrl-C always quits, whatever the mode.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match app.mode.clone() {
        Mode::Normal => handle_normal(app, key),
        Mode::Settings => handle_settings(app, key),
        Mode::Input { .. } => handle_input(app, key),
        Mode::ConfirmDelete { .. } => handle_confirm(app, key),
        Mode::PostAdd { .. } => handle_post_add(app, key),
    }
}

fn handle_post_add(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.continue_to_login(),
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => app.skip_login(),
        _ => {}
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    // Any keypress clears a lingering status message.
    app.status = None;
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Enter => app.activate(),
        KeyCode::Char('r') | KeyCode::Char('R') => app.manual_refresh(),
        KeyCode::Char('m') | KeyCode::Char('M') => app.toggle_compact(),
        KeyCode::Char('s') | KeyCode::Char('S') => app.open_settings(),
        _ => {}
    }
}

// The add / edit / delete actions live behind the settings menu so a stray
// keypress on the main list can't add, rename, or remove a profile. Esc backs
// out to the normal list.
fn handle_settings(app: &mut App, key: KeyEvent) {
    app.status = None;
    match key.code {
        KeyCode::Esc | KeyCode::Char('s') | KeyCode::Char('S') => app.cancel(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Char('a') | KeyCode::Char('A') => app.begin_add(),
        KeyCode::Char('e') | KeyCode::Char('E') => app.begin_rename(),
        KeyCode::Char('d') | KeyCode::Char('D') => app.begin_delete(),
        _ => {}
    }
}

fn handle_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.cancel(),
        KeyCode::Enter => app.commit_input(),
        KeyCode::Backspace => app.input_backspace(),
        KeyCode::Char(c) => app.input_push(c),
        _ => {}
    }
}

fn handle_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_delete(),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel(),
        _ => {}
    }
}
