//! Rendering. Pure function of `App` state onto a ratatui `Frame`.
//!
//! The UI is deliberately compact: a title line, the profile list, and a single
//! footer line that doubles as the prompt for add / rename / delete, so the
//! whole thing fits in a small inline viewport rather than taking over the
//! terminal.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::commands::humanize;

use super::app::{App, InputAction, Mode};

const ACCENT: Color = Color::Cyan;

/// Chrome that surrounds the profile list (borders + title + footer lines).
pub const CHROME_LINES: u16 = 6;
/// Each profile occupies this many rows in the list.
pub const ROWS_PER_PROFILE: u16 = 2;

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Min(2),    // profile list
            Constraint::Length(1), // footer / prompt
        ])
        .split(f.area());

    draw_title(f, chunks[0], app);
    draw_list(f, chunks[1], app);
    draw_footer(f, chunks[2], app);
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let active = app
        .profiles()
        .iter()
        .find(|p| p.active)
        .map(|p| p.name.as_str())
        .unwrap_or("none");
    let line = Line::from(vec![
        Span::styled(" Claude Accounts", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(format!("active: {active}"), Style::default().fg(ACCENT)),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .profiles()
        .iter()
        .map(|p| {
            let check = if p.active { "✓ " } else { "  " };
            let mut spans = vec![
                Span::styled(
                    check,
                    Style::default().fg(if p.active { Color::Green } else { Color::DarkGray }),
                ),
                Span::styled(
                    p.name.clone(),
                    Style::default().add_modifier(if p.active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
            ];
            if let Some(email) = &p.email {
                spans.push(Span::styled(
                    format!(" ({email})"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            let mut tags = Vec::new();
            if !p.exists {
                tags.push("MISSING DIR");
            }
            if !p.authenticated {
                tags.push("not signed in");
            }
            let detail = format!(
                "     last used: {}{}",
                humanize(p.last_used),
                if tags.is_empty() {
                    String::new()
                } else {
                    format!("   [{}]", tags.join(", "))
                }
            );
            ListItem::new(vec![
                Line::from(spans),
                Line::from(Span::styled(detail, Style::default().fg(Color::DarkGray))),
            ])
        })
        .collect();

    let mut state = ListState::default();
    if !app.profiles().is_empty() {
        state.select(Some(app.selected));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Profiles "))
        .highlight_style(
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, &mut state);
}

/// The footer line adapts to the current mode: keybindings in normal mode, an
/// inline text field when adding/renaming, and a y/n prompt when deleting.
fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let line = match &app.mode {
        Mode::Input { action, buffer } => {
            let label = match action {
                InputAction::Add => "add".to_string(),
                InputAction::Rename { from } => format!("rename '{from}'"),
            };
            Line::from(vec![
                Span::styled(format!(" {label} — name: "), Style::default().fg(ACCENT)),
                Span::styled(buffer.clone(), Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("▏", Style::default().fg(ACCENT)),
                Span::styled("  (enter ok · esc cancel)", Style::default().fg(Color::DarkGray)),
            ])
        }
        Mode::ConfirmDelete { name } => Line::from(vec![
            Span::styled(format!(" remove '{name}'? "), Style::default().fg(Color::Red)),
            Span::styled("directory kept  ", Style::default().fg(Color::DarkGray)),
            Span::styled("(y confirm · n cancel)", Style::default().fg(Color::DarkGray)),
        ]),
        Mode::Normal => {
            if let Some(status) = &app.status {
                Line::from(Span::styled(format!(" {status}"), Style::default().fg(ACCENT)))
            } else {
                Line::from(Span::styled(
                    " ↑↓ move · enter switch · a add · r rename · d delete · q quit",
                    Style::default().fg(Color::DarkGray),
                ))
            }
        }
    };
    f.render_widget(Paragraph::new(line), area);
}
