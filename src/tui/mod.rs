//! Interactive terminal UI: terminal lifecycle and the input event loop.

mod app;
mod ui;

use std::io::{self, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::error::Result;
use crate::manager::Manager;

use app::{App, Mode};

/// Entry point from `main` when no subcommand is given.
pub fn run(manager: &mut Manager) -> Result<()> {
    if manager.profiles().is_empty() {
        eprintln!("No profiles yet. Add one with:  claude-switcher add <name>");
        return Ok(());
    }

    let mut terminal = setup_terminal()?;
    let app = App::new(manager);
    let result = event_loop(&mut terminal, app);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut app: App,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

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
