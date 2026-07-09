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
use crate::profile::Profile;

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
/// In the compact (minimal) view each profile is a single line: alias, 5-hour
/// bar, and reset time.
pub const COMPACT_ROWS_PER_PROFILE: u16 = 1;
/// The widest the UI is ever rendered. On a wide terminal the bordered frame is
/// clamped to this (and left-aligned) so it doesn't stretch out unattractively;
/// it's sized to hold the longest line (a usage bar plus its reset phrase).
pub const MAX_WIDTH: u16 = 80;

/// Clamp a full-terminal rect to at most `MAX_WIDTH` columns, left-aligned at
/// the terminal origin. Narrower terminals are returned unchanged.
fn clamp_width(area: Rect) -> Rect {
    Rect {
        width: area.width.min(MAX_WIDTH),
        ..area
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Min(2),    // profile list
            Constraint::Length(1), // footer / prompt
        ])
        .split(clamp_width(f.area()));

    draw_title(f, chunks[0], app);
    draw_list(f, chunks[1], app);
    draw_footer(f, chunks[2], app);
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    // Single left-aligned line: title, last-updated time, then the auto-refresh
    // toggle just after it (a focusable "row" — Enter toggles it). When focused
    // it highlights here and no profile row is, so the list stays easy to read.
    let toggle_style = if app.header_focused() {
        Style::default().bg(SELECTION_BG).fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        secondary()
    };
    let marker = if app.header_focused() { "› " } else { "" };
    let line = Line::from(vec![
        Span::styled(" Claude Switcher", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled(app.updated_label(), secondary()),
        Span::raw("    "),
        Span::styled(format!("{marker}{}", app.auto_refresh_label()), toggle_style),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = if app.compact() {
        // Pad names to the widest so the bars line up across rows.
        let name_width = app.profiles().iter().map(|p| p.name.chars().count()).max().unwrap_or(0);
        app.profiles()
            .iter()
            .map(|p| ListItem::new(compact_line(p, app.usage(&p.name), name_width)))
            .collect()
    } else {
        app.profiles()
            .iter()
            .map(|p| detailed_item(p, app.usage(&p.name)))
            .collect()
    };

    // Highlight a profile row only when one is selected; when the header
    // Refresh control is focused, nothing here is highlighted.
    let mut state = ListState::default();
    if let Some(i) = app.selected_profile_index() {
        state.select(Some(i));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Accounts "))
        // Keep the text fully legible: just a marker + bold, no washed-out
        // background or reversed colors.
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("› ")
        // Reserve the marker column always, so rows don't shift when focus
        // moves between the header and the list.
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(list, area, &mut state);
}

/// The full four-line profile row: name (+ identity), 5h bar, 7d bar, last-used.
fn detailed_item(p: &Profile, state: Option<&UsageState>) -> ListItem<'static> {
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
    append_usage_lines(&mut lines, state);
    lines.push(Line::from(Span::styled(detail, secondary())));
    ListItem::new(lines)
}

/// A single compact row: `✓ alias  ██░░  25%  resets in 3h 36m (2:50pm)`. The
/// alias is padded to `name_width` so bars align across rows; only the 5-hour
/// window is shown.
fn compact_line(p: &Profile, state: Option<&UsageState>, name_width: usize) -> Line<'static> {
    let check = if p.active { "✓ " } else { "  " };
    let mut spans = vec![
        Span::styled(
            check,
            Style::default().fg(if p.active { Color::Green } else { Color::DarkGray }),
        ),
        Span::styled(
            format!("{:<name_width$}", p.name),
            Style::default().add_modifier(if p.active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
        Span::raw("  "),
    ];
    spans.extend(compact_usage_spans(state));
    Line::from(spans)
}

/// The usage portion of a compact row: the 5-hour bar with its reset time, or a
/// loading / unavailable / n-a placeholder.
fn compact_usage_spans(state: Option<&UsageState>) -> Vec<Span<'static>> {
    let dim = secondary();
    match state {
        Some(UsageState::Ready(u)) | Some(UsageState::Cached(u)) => match u.five_hour.as_ref() {
            Some(w) => {
                let pct = w.utilization.round() as i64;
                let color = threshold_color(pct);
                vec![
                    Span::styled(crate::usage::bar(w.utilization, 16), Style::default().fg(color)),
                    Span::styled(format!(" {pct:>3}%"), Style::default().fg(color)),
                    Span::styled(format!("  {}", reset_phrase(w)), dim),
                ]
            }
            None => vec![Span::styled("5h n/a", dim)],
        },
        Some(UsageState::Loading) | None => vec![Span::styled("usage …", dim)],
        Some(UsageState::Unavailable) => vec![Span::styled("usage unavailable", dim)],
    }
}

/// Append the two usage rows (5-hour and 7-day) for a profile, each a labeled
/// progress bar with its reset time — or loading / unavailable placeholders.
/// Always adds exactly two lines so every profile has a fixed height.
fn append_usage_lines(lines: &mut Vec<Line<'static>>, state: Option<&UsageState>) {
    let dim = secondary();
    match state {
        Some(UsageState::Ready(u)) | Some(UsageState::Cached(u)) => {
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

/// Widest relative-reset countdown we render (e.g. "resets in 23h 59m"). The
/// countdown is left-padded to this so the "(clock)" that follows lines up in
/// the same column on a profile's 5h and 7d rows.
const RESET_REL_WIDTH: usize = 17;

/// e.g. "resets in 3h 36m  (2:50pm)". The countdown is padded to a fixed width
/// so the parenthesized clock time aligns vertically across the two rows.
fn reset_phrase(window: &crate::usage::Window) -> String {
    match (crate::usage::resets_in(window), crate::usage::reset_clock(window)) {
        (Some(rel), Some(clock)) => format!("{rel:<w$} ({clock})", w = RESET_REL_WIDTH),
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
                // switch/rename/delete a profile, and Enter just refreshes. The
                // view toggle sits just before quit, which stays last.
                let base = if app.header_focused() {
                    " ↑↓ move · enter toggle auto-refresh · a add · r refresh"
                } else if app.selected_profile().is_some_and(|p| p.active) {
                    // Already on the active profile: a (second) Enter closes.
                    " ↑↓ move · enter close · a add · e edit · d delete · r refresh"
                } else {
                    " ↑↓ move · enter switch · a add · e edit · d delete · r refresh"
                };
                // 'm' flips between the full and compact (minimal) views. Kept
                // to 8 cols so the longest footer variant still fits the width.
                let view = if app.compact() { " · m max" } else { " · m min" };
                Line::from(Span::styled(format!("{base}{view} · q quit"), secondary()))
            }
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::Manager;
    use crate::paths::Paths;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tempfile::tempdir;

    /// Flatten a rendered buffer into per-row strings for substring assertions.
    fn rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buf = terminal.backend().buffer();
        let width = buf.area.width as usize;
        buf.content()
            .chunks(width)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn compact_view_renders_one_line_per_profile() {
        let dir = tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("home"), dir.path().join("cfg"));
        std::fs::create_dir_all(&paths.home).unwrap();
        let mut mgr = Manager::load(paths).unwrap();
        mgr.add("paul-nhost", None).unwrap();
        mgr.add("work", None).unwrap();

        let mut app = App::new(&mut mgr);
        app.toggle_compact();

        let mut terminal = Terminal::new(TestBackend::new(MAX_WIDTH, 12)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let rows = rows(&terminal);

        // The column a needle starts in (chars, not bytes: the border/check
        // glyphs are multi-byte, so a byte offset would misreport the column).
        let col = |needle: &str| {
            rows.iter().find_map(|r| r.find(needle).map(|b| r[..b].chars().count()))
        };

        // Each profile is a single row; names are padded to the widest so their
        // bars begin at the same column.
        let paul = rows.iter().find(|r| r.contains("paul-nhost")).unwrap();
        let work = rows.iter().find(|r| r.contains("work")).unwrap();
        assert_ne!(paul, work, "each profile occupies its own line");
        assert_eq!(col("paul-nhost"), col("work"), "aliases start in the same column");
    }

    #[test]
    fn clamp_width_caps_wide_and_preserves_narrow() {
        let wide = Rect { x: 0, y: 0, width: 200, height: 24 };
        let clamped = clamp_width(wide);
        assert_eq!(clamped.width, MAX_WIDTH);
        // Height, position, and origin are untouched — only width is capped.
        assert_eq!((clamped.x, clamped.y, clamped.height), (0, 0, 24));

        let narrow = Rect { x: 0, y: 0, width: 50, height: 24 };
        assert_eq!(clamp_width(narrow), narrow);
    }
}
