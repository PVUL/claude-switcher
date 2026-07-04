//! Rendering. Pure function of `App` state onto a ratatui `Frame`.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::commands::humanize;

use super::app::{App, InputAction, Mode};

const ACCENT: Color = Color::Cyan;

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Min(6),    // profile list
            Constraint::Length(3), // footer / keybindings
        ])
        .split(f.area());

    draw_title(f, chunks[0], app);
    draw_list(f, chunks[1], app);
    draw_footer(f, chunks[2], app);

    match &app.mode {
        Mode::Input { action, buffer } => draw_input_popup(f, action, buffer),
        Mode::ConfirmDelete { name } => draw_confirm_popup(f, name),
        Mode::Normal => {}
    }
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let active = app
        .profiles()
        .iter()
        .find(|p| p.active)
        .map(|p| p.name.as_str())
        .unwrap_or("none");
    let line = Line::from(vec![
        Span::styled("  Claude Accounts", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("   "),
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

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let keys = "ENTER switch   A add   R rename   D delete   Q quit";
    let status = app.status.clone().unwrap_or_else(|| keys.to_string());
    let style = if app.status.is_some() {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let p = Paragraph::new(Line::from(Span::styled(status, style)))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_input_popup(f: &mut Frame, action: &InputAction, buffer: &str) {
    let title = match action {
        InputAction::Add => " Add profile ".to_string(),
        InputAction::Rename { from } => format!(" Rename '{from}' "),
    };
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);
    let text = vec![
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::raw("  name: "),
            Span::styled(buffer, Style::default().fg(ACCENT)),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  ENTER confirm    ESC cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(title).border_style(Style::default().fg(ACCENT)));
    f.render_widget(p, area);
}

fn draw_confirm_popup(f: &mut Frame, name: &str) {
    let area = centered_rect(52, 24, f.area());
    f.render_widget(Clear, area);
    let text = vec![
        Line::from(Span::raw("")),
        Line::from(Span::raw(format!("  Remove profile '{name}' from claudesub?"))),
        Line::from(Span::styled(
            "  (the directory on disk is kept)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Y confirm    N / ESC cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Confirm delete ")
            .border_style(Style::default().fg(Color::Red))
            .title_alignment(Alignment::Left),
    );
    f.render_widget(p, area);
}

/// A rectangle centered within `r`, sized as a percentage of it.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
