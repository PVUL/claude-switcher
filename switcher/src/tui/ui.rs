//! Rendering. Pure function of `App` state onto a ratatui `Frame`.
//!
//! The UI is deliberately compact: the profile list (its top border carries the
//! header — app name, last-updated time, auto-refresh toggle) and a single
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

/// Chrome around the profile list: the list block's two borders (its top border
/// doubles as the header) plus the one footer line.
pub const CHROME_LINES: u16 = 3;
/// Each profile occupies this many rows: the name row (with last-used aligned
/// right), then the 5h and 7d usage bars.
pub const ROWS_PER_PROFILE: u16 = 3;
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
            Constraint::Min(2),    // profile list (top border carries the header)
            Constraint::Length(1), // footer / prompt
        ])
        .split(clamp_width(f.area()));

    draw_list(f, chunks[0], app);
    draw_footer(f, chunks[1], app);
}

/// The header, rendered onto the list block's top border: the app name, the
/// last-updated time, and the auto-refresh toggle. The toggle is a focusable
/// "row" (Enter toggles it); when focused it highlights here and no profile row
/// is, so the list stays easy to read. Folding this onto the border drops the
/// separate title bar, saving two lines of height.
fn header_title(app: &App) -> Line<'static> {
    let toggle_style = if app.header_focused() {
        Style::default().bg(SELECTION_BG).fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        secondary()
    };
    let marker = if app.header_focused() { "› " } else { "" };
    Line::from(vec![
        Span::styled(" Claude Switcher ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {} ", app.updated_label()), secondary()),
        Span::styled(format!(" {marker}{} ", app.auto_refresh_label()), toggle_style),
    ])
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
        // Width available to a row's text: the list area minus the block's two
        // borders and the always-reserved 2-col highlight gutter ("› "). Used
        // to flush the last-used text to the right edge of the name line.
        let content_width = area.width.saturating_sub(4) as usize;
        app.profiles()
            .iter()
            .map(|p| detailed_item(p, app.usage(&p.name), content_width))
            .collect()
    };

    // Highlight a profile row only when one is selected; when the header
    // Refresh control is focused, nothing here is highlighted.
    let mut state = ListState::default();
    if let Some(i) = app.selected_profile_index() {
        state.select(Some(i));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(header_title(app)))
        // Keep the text fully legible: just a marker + bold, no washed-out
        // background or reversed colors.
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("› ")
        // Reserve the marker column always, so rows don't shift when focus
        // moves between the header and the list.
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(list, area, &mut state);
}

/// The three-line profile row: the name (+ identity) with the last-used time
/// (and any status tags) flushed to the right edge on the same line, then the
/// 5h and 7d usage bars. `width` is the row's usable text width, used to pad
/// the name line so the right-hand text sits against the right border.
fn detailed_item(p: &Profile, state: Option<&UsageState>, width: usize) -> ListItem<'static> {
    let check = if p.active { "✓ " } else { "  " };
    let mut left = vec![
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
        left.push(Span::styled(format!(" ({id})"), secondary()));
    }

    // Right-aligned trailer: any status tags, then the last-used time. Folding
    // this onto the name line drops a whole row from every profile's height.
    let mut tags = Vec::new();
    if !p.exists {
        tags.push("MISSING DIR");
    }
    if !p.authenticated {
        tags.push("not signed in");
    }
    if p.email_mismatch().is_some() {
        tags.push("WRONG ACCOUNT");
    }
    let right = if tags.is_empty() {
        format!("last used {}", humanize(p.last_used))
    } else {
        format!("[{}] · last used {}", tags.join(", "), humanize(p.last_used))
    };

    // Fill the gap between the left text and the right trailer with spaces so
    // the trailer is right-aligned; keep at least one space if the row is tight.
    let left_cols: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let pad = width.saturating_sub(left_cols + right.chars().count()).max(1);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, secondary()));

    let mut lines = vec![Line::from(spans)];
    append_usage_lines(&mut lines, state);
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
/// loading / unavailable placeholder. A fetched-but-missing 5-hour window is
/// shown as an empty 0% bar (a missing window means no usage on record) rather
/// than "n/a".
fn compact_usage_spans(state: Option<&UsageState>) -> Vec<Span<'static>> {
    let dim = secondary();
    match state {
        Some(UsageState::Ready(u)) | Some(UsageState::Cached(u)) => {
            let util = u.five_hour.as_ref().map_or(0.0, |w| w.utilization);
            let pct = util.round() as i64;
            let color = threshold_color(pct);
            let mut spans = vec![
                Span::styled(crate::usage::bar(util, 16), Style::default().fg(color)),
                Span::styled(format!(" {pct:>3}%"), Style::default().fg(color)),
            ];
            // A real window carries a reset time; a missing one (0%) has
            // nothing to count down to, so we leave the reset phrase off.
            if let Some(w) = u.five_hour.as_ref() {
                spans.push(Span::styled(format!("  {}", reset_phrase(w)), dim));
            }
            spans
        }
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
    // A window we couldn't fetch is treated as freshly reset — an empty 0% bar
    // rather than "n/a", since a missing window means no usage on record.
    let util = window.map_or(0.0, |w| w.utilization);
    let pct = util.round() as i64;
    let color = threshold_color(pct);
    let mut spans = vec![
        Span::styled(format!("     {label} "), dim),
        Span::styled(crate::usage::bar(util, 16), Style::default().fg(color)),
        Span::styled(format!(" {pct:>3}%"), Style::default().fg(color)),
    ];
    // A real window carries a reset time; a missing one (0%) has nothing to
    // count down to, so we leave the reset phrase off.
    if let Some(w) = window {
        spans.push(Span::styled(format!("  {}", reset_phrase(w)), dim));
    }
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
        Mode::PostAdd { name } => Line::from(vec![
            Span::styled(format!(" added '{name}' "), Style::default().fg(Color::Green)),
            Span::styled("— sign in with Claude?  ", Style::default().fg(ACCENT)),
            Span::styled("(enter launch login · esc skip)", secondary()),
        ]),
        // The settings menu surfaces the destructive actions kept off the main
        // list; esc backs out.
        Mode::Settings => Line::from(vec![
            Span::styled(" settings — ", Style::default().fg(ACCENT)),
            Span::styled("↑↓ move · a add · e edit · d delete · esc back", secondary()),
        ]),
        Mode::Normal => {
            if let Some(status) = &app.status {
                Line::from(Span::styled(format!(" {status}"), Style::default().fg(ACCENT)))
            } else {
                // Keys depend on focus: on the header Refresh control Enter just
                // refreshes; on a profile it switches (or closes the active
                // one). Add / edit / delete live behind `s settings`.
                let base = if app.header_focused() {
                    " ↑↓ move · enter toggle auto-refresh · r refresh"
                } else if app.selected_profile().is_some_and(|p| p.active) {
                    // Already on the active profile: a (second) Enter closes.
                    " ↑↓ move · enter close · r refresh"
                } else {
                    " ↑↓ move · enter switch · r refresh"
                };
                // 'm' flips between the full and compact (minimal) views. Kept
                // to 8 cols so the longest footer variant still fits the width.
                let view = if app.compact() { " · m max" } else { " · m min" };
                Line::from(Span::styled(format!("{base}{view} · s settings · q quit"), secondary()))
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
    fn detailed_view_puts_last_used_on_the_name_line() {
        let dir = tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("home"), dir.path().join("cfg"));
        std::fs::create_dir_all(&paths.home).unwrap();
        let mut mgr = Manager::load(paths).unwrap();
        mgr.add("paul-nhost", None).unwrap();

        let app = App::new(&mut mgr);
        let mut terminal = Terminal::new(TestBackend::new(MAX_WIDTH, 12)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let rows = rows(&terminal);

        // The name and its last-used trailer share one row (no separate line).
        let name_row = rows.iter().find(|r| r.contains("paul-nhost")).unwrap();
        assert!(name_row.contains("last used"), "last-used sits on the name line");
        // And it is flushed right: the name is left of the trailer on that row.
        let name_col = name_row.find("paul-nhost").map(|b| name_row[..b].chars().count());
        let used_col = name_row.find("last used").map(|b| name_row[..b].chars().count());
        assert!(used_col > name_col, "last-used is right of the name");
        // No other row repeats it — the dedicated last-used line is gone.
        assert_eq!(rows.iter().filter(|r| r.contains("last used")).count(), 1);
    }

    #[test]
    fn header_sits_on_the_list_top_border() {
        let dir = tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("home"), dir.path().join("cfg"));
        std::fs::create_dir_all(&paths.home).unwrap();
        let mut mgr = Manager::load(paths).unwrap();
        mgr.add("paul-nhost", None).unwrap();

        let app = App::new(&mut mgr);
        let mut terminal = Terminal::new(TestBackend::new(MAX_WIDTH, 12)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let rows = rows(&terminal);

        // The header rides the list block's top border (row 0), not a separate
        // title bar, so we don't spend two extra lines on it.
        assert!(rows[0].contains("Claude Switcher"), "app name on the top border");
        assert!(rows[0].contains("auto-refresh"), "toggle on the top border");
        // The old dedicated " Accounts " title is gone.
        assert!(!rows.iter().any(|r| r.contains("Accounts")), "no Accounts title");
    }

    #[test]
    fn missing_window_renders_as_zero_percent_not_na() {
        // Detailed row: a window we couldn't fetch reads as an empty 0% bar.
        let text: String = window_bar_line("5h", None, None)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("0%"), "missing window shows 0%, got {text:?}");
        assert!(!text.contains("n/a"), "no n/a placeholder, got {text:?}");

        // Compact row: a Ready snapshot with no 5-hour window does the same.
        let state = UsageState::Ready(crate::usage::Usage {
            five_hour: None,
            seven_day: None,
            seven_day_opus: None,
        });
        let text: String = compact_usage_spans(Some(&state))
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("0%"), "compact shows 0%, got {text:?}");
        assert!(!text.contains("n/a"), "compact has no n/a, got {text:?}");
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
