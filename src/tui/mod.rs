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
use ui::{CHROME_LINES, ROWS_PER_PROFILE};

/// Entry point from `main` when no subcommand is given.
pub fn run(manager: &mut Manager) -> Result<()> {
    if manager.profiles().is_empty() {
        eprintln!("No profiles yet. Add one with:  claude-switcher add <name>");
        return Ok(());
    }

    let height = viewport_height(manager.profiles().len());
    let mut terminal = setup_terminal(height)?;
    let app = App::new(manager);
    let result = event_loop(&mut terminal, app);
    restore_terminal(&mut terminal)?;
    result
}

/// Size the inline viewport to the content, capped so it never dominates the
/// terminal. The list scrolls if there are more profiles than fit.
fn viewport_height(profiles: usize) -> u16 {
    let content = CHROME_LINES + ROWS_PER_PROFILE * profiles.max(1) as u16;
    content.clamp(CHROME_LINES + ROWS_PER_PROFILE, 28)
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

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut app: App,
) -> Result<()> {
    loop {
        app.pump_usage();
        terminal.draw(|f| ui::draw(f, &app))?;

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
        handle_key(&mut app, key);
        if app.should_quit {
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
        Mode::Input { .. } => handle_input(app, key),
        Mode::ConfirmDelete { .. } => handle_confirm(app, key),
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    // Any keypress clears a lingering status message.
    app.status = None;
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Enter => app.switch_selected(),
        KeyCode::Char('a') | KeyCode::Char('A') => app.begin_add(),
        KeyCode::Char('r') | KeyCode::Char('R') => app.begin_rename(),
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
