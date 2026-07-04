//! Rendering. Pure function of `App` state onto a ratatui `Frame`.
//!
//! The UI is deliberately compact: a title line, the profile list, and a single
//! footer line that doubles as the prompt for add / rename / delete, so the
//! whole thing fits in a small inline viewport rather than taking over the
//! terminal.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::commands::humanize;

use super::app::{App, InputAction, Mode, UsageState};

const ACCENT: Color = Color::Cyan;
/// Muted dark-gray background for the selected row (256-color index).
const SELECTION_BG: Color = Color::Indexed(237);

/// Style for secondary text. Dims the terminal's *own* foreground rather than
/// using a fixed grey, so it stays legible on any background (a fixed grey
/// clashes with grey terminal themes).
fn secondary() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Chrome that surrounds the profile list (borders + title + footer lines).
pub const CHROME_LINES: u16 = 6;
/// Each profile occupies this many rows (name, 5h bar, 7d bar, last-used).
pub const ROWS_PER_PROFILE: u16 = 4;

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
    // Single left-aligned line: title, last-updated time, then the auto-refresh
    // toggle just after it (a focusable "row" — Enter toggles it). The ↻ icon is
    // a visual cue. When focused it highlights here and no profile row is, so
    // the list stays easy to read.
    let toggle_style = if app.header_focused() {
        Style::default().bg(SELECTION_BG).fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        secondary()
    };
    let marker = if app.header_focused() { "› " } else { "" };
    let line = Line::from(vec![
        Span::styled(" Claude Accounts", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled(app.updated_label(), secondary()),
        Span::raw("    "),
        Span::styled(format!("{marker}↻ {}", app.auto_refresh_label()), toggle_style),
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
            if let Some(id) = p.identity() {
                spans.push(Span::styled(format!(" ({id})"), secondary()));
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
            let mut lines = vec![Line::from(spans)];
            append_usage_lines(&mut lines, app.usage(&p.name));
            lines.push(Line::from(Span::styled(detail, secondary())));
            ListItem::new(lines)
        })
        .collect();

    // Highlight a profile row only when one is selected; when the header
    // Refresh control is focused, nothing here is highlighted.
    let mut state = ListState::default();
    if let Some(i) = app.selected_profile_index() {
        state.select(Some(i));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Profiles "))
        // Keep the text fully legible: just a marker + bold, no washed-out
        // background or reversed colors.
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("› ")
        // Reserve the marker column always, so rows don't shift when focus
        // moves between the header and the list.
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(list, area, &mut state);
}

/// Append the two usage rows (5-hour and 7-day) for a profile, each a labeled
/// progress bar with its reset time — or loading / unavailable placeholders.
/// Always adds exactly two lines so every profile has a fixed height.
fn append_usage_lines(lines: &mut Vec<Line<'static>>, state: Option<&UsageState>) {
    let dim = secondary();
    match state {
        Some(UsageState::Ready(u)) => {
            lines.push(window_bar_line("5h", u.five_hour.as_ref(), None));
            let opus = u.seven_day_opus.as_ref().filter(|w| w.utilization > 0.0);
            lines.push(window_bar_line("7d", u.seven_day.as_ref(), opus));
        }
        Some(UsageState::Loading) | None => {
            lines.push(Line::from(Span::styled("     usage …", dim)));
            lines.push(Line::from(Span::raw("")));
        }
        Some(UsageState::Unavailable) => {
            lines.push(Line::from(Span::styled("     usage unavailable", dim)));
            lines.push(Line::from(Span::raw("")));
        }
    }
}

fn window_bar_line(
    label: &str,
    window: Option<&crate::usage::Window>,
    opus: Option<&crate::usage::Window>,
) -> Line<'static> {
    let dim = secondary();
    let Some(w) = window else {
        return Line::from(Span::styled(format!("     {label} n/a"), dim));
    };
    let pct = w.utilization.round() as i64;
    let color = threshold_color(pct);
    let mut spans = vec![
        Span::styled(format!("     {label} "), dim),
        Span::styled(crate::usage::bar(w.utilization, 16), Style::default().fg(color)),
        Span::styled(format!(" {pct:>3}%"), Style::default().fg(color)),
        Span::styled(format!("  {}", reset_phrase(w)), dim),
    ];
    if let Some(o) = opus {
        spans.push(Span::styled(
            format!("  · opus {}%", o.utilization.round() as i64),
            dim,
        ));
    }
    Line::from(spans)
}

fn threshold_color(pct: i64) -> Color {
    if pct >= 90 {
        Color::Red
    } else if pct >= 70 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// e.g. "resets in 3h 36m (14:50)".
fn reset_phrase(window: &crate::usage::Window) -> String {
    match (crate::usage::resets_in(window), crate::usage::reset_clock(window)) {
        (Some(rel), Some(clock)) => format!("{rel} ({clock})"),
        (Some(rel), None) => rel,
        _ => String::new(),
    }
}

/// The footer line adapts to the current mode: keybindings in normal mode, an
/// inline text field when adding/renaming, and a y/n prompt when deleting.
fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let line = match &app.mode {
        Mode::Input { action, buffer } => {
            let label = match action {
                InputAction::Add => "add".to_string(),
                InputAction::Rename { from } => format!("edit '{from}'"),
            };
            Line::from(vec![
                Span::styled(format!(" {label} — name: "), Style::default().fg(ACCENT)),
                Span::styled(buffer.clone(), Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("▏", Style::default().fg(ACCENT)),
                Span::styled("  (enter ok · esc cancel)", secondary()),
            ])
        }
        Mode::ConfirmDelete { name } => Line::from(vec![
            Span::styled(format!(" remove '{name}'? "), Style::default().fg(Color::Red)),
            Span::styled("directory kept  ", secondary()),
            Span::styled("(y confirm · n cancel)", secondary()),
        ]),
        Mode::Normal => {
            if let Some(status) = &app.status {
                Line::from(Span::styled(format!(" {status}"), Style::default().fg(ACCENT)))
            } else {
                // Keys depend on focus: on the header Refresh control you can't
                // switch/rename/delete a profile, and Enter just refreshes.
                let keys = if app.header_focused() {
                    " ↑↓ move · enter toggle auto-refresh · r refresh · a add · q quit"
                } else {
                    " ↑↓ move · enter switch · a add · e edit · d delete · r refresh · q quit"
                };
                Line::from(Span::styled(keys, secondary()))
            }
        }
    };
    f.render_widget(Paragraph::new(line), area);
}
