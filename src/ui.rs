use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Wrap,
};

use crate::app::{App, Mode};
use crate::model::{Agent, SessionStatus, relative_time};

const CLAUDE: Color = Color::Rgb(232, 142, 79);
const CODEX: Color = Color::Rgb(55, 190, 184);
const CURSOR: Color = Color::Rgb(139, 126, 255);
const PI: Color = Color::Rgb(244, 202, 90);
const OPENCODE: Color = Color::Rgb(95, 205, 125);
const MUTED: Color = Color::Rgb(120, 128, 140);
const ACCENT: Color = Color::Rgb(122, 162, 247);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    draw_body(frame, app, rows[0]);
    draw_footer(frame, app, rows[1]);

    match app.mode {
        Mode::Search => draw_search(frame, app),
        Mode::Filter => draw_filter(frame, app),
        Mode::Help => draw_help(frame),
        Mode::Handoff => draw_handoff(frame, app),
        Mode::ConfirmLaunch => draw_confirm(frame, app),
        Mode::Normal => {}
    }

    if let Some(toast) = &app.toast {
        draw_toast(frame, &toast.message, toast.is_error);
    }
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.width >= 120 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(rows[0]);
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
        draw_sessions(frame, app, Agent::Codex, top[0]);
        draw_sessions(frame, app, Agent::Claude, top[1]);
        draw_sessions(frame, app, Agent::Cursor, top[2]);
        draw_sessions(frame, app, Agent::Pi, bottom[0]);
        draw_sessions(frame, app, Agent::OpenCode, bottom[1]);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(area);
        let first = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        let second = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
        draw_sessions(frame, app, Agent::Codex, first[0]);
        draw_sessions(frame, app, Agent::Claude, first[1]);
        draw_sessions(frame, app, Agent::Cursor, second[0]);
        draw_sessions(frame, app, Agent::Pi, second[1]);
        draw_sessions(frame, app, Agent::OpenCode, rows[2]);
    }
}

fn draw_sessions(frame: &mut Frame, app: &App, agent: Agent, area: Rect) {
    let indices = app.visible_indices_for(agent);
    let focused = agent == app.active_agent;
    let border_style = if focused {
        agent_style(agent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let title = format!(
        " {}{} ({}) ",
        if focused { "● " } else { "" },
        agent.label(),
        indices.len()
    );
    if indices.is_empty() {
        let message = if app.agent_session_count(agent) == 0 {
            "No sessions in this folder."
        } else {
            "No matching sessions."
        };
        frame.render_widget(
            Paragraph::new(message)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(title),
                )
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let rows = indices.iter().map(|index| {
        let session = &app.sessions[*index];
        Row::new(vec![
            Cell::from(Span::styled(
                session.status.glyph(),
                status_style(session.status),
            )),
            Cell::from(session.title.clone()),
            Cell::from(relative_time(session.last_activity)),
        ])
    });
    let widths = [
        Constraint::Length(2),
        Constraint::Min(16),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["", "Session", "Activity"])
                .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD))
                .bottom_margin(1),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title),
        )
        .row_highlight_style(if focused {
            Style::default()
                .bg(Color::Rgb(45, 50, 65))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        })
        .highlight_symbol(if focused { "▌" } else { " " });
    let mut state = TableState::default().with_selected(focused.then(|| app.selection_for(agent)));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let full_folder = app
        .scan_options
        .scope
        .as_ref()
        .map(|scope| format!(" folder: {} ", scope.display()))
        .unwrap_or_else(|| " all folders ".to_owned());
    let max_folder_width = (area.width.saturating_mul(45) / 100).max(1);
    let folder = truncate_left(&full_folder, usize::from(max_folder_width));
    let folder_width = folder.chars().count() as u16;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(folder_width)])
        .split(area);
    let content = match app.mode {
        Mode::Normal if columns[0].width < 100 => " ↑↓   Ctrl+←→   ↵ resume   h handoff ",
        Mode::Normal => {
            " ↑/↓ session   Ctrl+arrows panel   ↵ resume   h handoff   x launch   / search   f filter   ? help   q quit "
        }
        Mode::Search => " type to search   ↵ keep   Esc clear ",
        Mode::Filter => " ↑↓ field   ←→ choose   type values   ↵ apply   Esc cancel ",
        Mode::Help => " Esc close ",
        Mode::Handoff => " ↑↓ scroll   c copy   w write   x choose receiving agent   Esc close ",
        Mode::ConfirmLaunch => " ←→ choose agent   ↵ launch with handoff   Esc review ",
    };
    let footer_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(180, 190, 210));
    frame.render_widget(Block::default().style(footer_style), area);
    frame.render_widget(Paragraph::new(content).style(footer_style), columns[0]);
    frame.render_widget(
        Paragraph::new(folder)
            .alignment(Alignment::Right)
            .style(footer_style),
        columns[1],
    );
}

fn truncate_left(value: &str, max_width: usize) -> String {
    if value.chars().count() <= max_width {
        return value.to_owned();
    }
    if max_width <= 1 {
        return "…".chars().take(max_width).collect();
    }
    let tail = value
        .chars()
        .rev()
        .take(max_width - 1)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}

fn draw_search(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 70, 3);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!("{}█", app.search)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .title(" Search all session context "),
        ),
        area,
    );
}

fn draw_filter(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 64, 12);
    frame.render_widget(Clear, area);
    let values = [
        ("Project", app.draft_filters.project.clone()),
        ("Repository", app.draft_filters.repository.clone()),
        ("Recency", app.draft_filters.recency.label().to_owned()),
    ];
    let items = values.iter().enumerate().map(|(index, (label, value))| {
        let marker = if index == app.filter_field {
            "›"
        } else {
            " "
        };
        let value = if value.is_empty() { "any" } else { value };
        ListItem::new(Line::from(vec![
            Span::styled(format!("{marker} {label:<12}"), Style::default().fg(MUTED)),
            Span::styled(
                value,
                if index == app.filter_field {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ),
        ]))
    });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .title(" Filter sessions "),
        ),
        area,
    );
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame.area(), 76, 22);
    frame.render_widget(Clear, area);
    let help = "\
Navigation\n\
  Ctrl+arrows  move panel focus      ↑/k, ↓/j     select session\n\
  Home, G      first / last session\n\n\
Actions\n\
  Enter         resume in its agent   x             cross-launch agent\n\
  h             preview handoff       r             rescan local stores\n\n\
Find\n\
  /             search all context    f             structured filters\n\
  Esc           clear search/filters\n\n\
Handoff\n\
  c             copy Markdown         w             save in working directory\n\
  x             choose any other agent for the reviewed package\n\n\
Scope\n\
  By default only the exact current folder is shown. Use rejoin --all for all folders.\n\n\
Status uses shape and text as well as color. Active detection combines process\n\
arguments with the newest session in each live agent workspace.";
    frame.render_widget(
        Paragraph::new(help)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .title(" Help "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_handoff(frame: &mut Frame, app: &App) {
    let area = if frame.area().width < 120 {
        frame.area().inner(Margin::new(2, 1))
    } else {
        centered(frame.area(), 82, 84)
    };
    frame.render_widget(Clear, area);
    let markdown = app
        .handoff
        .as_ref()
        .map(|handoff| handoff.markdown.as_str())
        .unwrap_or("Handoff unavailable.");
    frame.render_widget(
        Paragraph::new(markdown)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .title(" Agent-neutral handoff · review before sharing "),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.handoff_scroll, 0)),
        area,
    );
    let line_count = markdown.lines().count();
    if line_count > area.height.saturating_sub(2) as usize {
        let mut state = ScrollbarState::new(line_count).position(app.handoff_scroll as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(Margin::new(0, 1)),
            &mut state,
        );
    }
}

fn draw_confirm(frame: &mut Frame, app: &App) {
    let Some(session) = app.selected_session() else {
        return;
    };
    let Some(target) = app.launch_target else {
        return;
    };
    let area = centered(frame.area(), 64, 9);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("Launch "),
                Span::styled(
                    target.label(),
                    agent_style(target).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" in:"),
            ]),
            Line::from(Span::styled(
                session.cwd.display().to_string(),
                Style::default().fg(MUTED),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "←  choose receiving agent  →",
                Style::default().fg(ACCENT),
            )),
            Line::from("The reviewed handoff will be supplied as its starting context."),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .title(" Confirm cross-agent launch "),
        ),
        area,
    );
}

fn draw_toast(frame: &mut Frame, message: &str, is_error: bool) {
    let width = (message.chars().count() as u16 + 4)
        .min(frame.area().width.saturating_sub(4))
        .max(20);
    let area = Rect::new(
        frame.area().right().saturating_sub(width + 2),
        frame.area().bottom().saturating_sub(4),
        width,
        3,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).border_style(
                Style::default().fg(if is_error { Color::Red } else { Color::Green }),
            )),
        area,
    );
}

fn centered(area: Rect, width_percent: u16, height: u16) -> Rect {
    let width = area.width.saturating_mul(width_percent).saturating_div(100);
    let height = if height <= 100 {
        area.height.saturating_mul(height).saturating_div(100)
    } else {
        height.min(area.height)
    };
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.max(1),
        height.max(1),
    )
}

fn agent_style(agent: Agent) -> Style {
    Style::default().fg(match agent {
        Agent::Claude => CLAUDE,
        Agent::Codex => CODEX,
        Agent::Cursor => CURSOR,
        Agent::Pi => PI,
        Agent::OpenCode => OPENCODE,
    })
}

fn status_style(status: SessionStatus) -> Style {
    Style::default().fg(match status {
        SessionStatus::Active => Color::Green,
        SessionStatus::Idle => Color::Yellow,
        SessionStatus::Stale => MUTED,
        SessionStatus::Error => Color::Red,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::Mode;
    use crate::model::{Filters, Handoff};
    use crate::scanner::ScanOptions;

    fn app() -> App {
        App {
            sessions: vec![crate::model::Session {
                id: "session-1".to_owned(),
                agent: Agent::Claude,
                project: "rejoin".to_owned(),
                repository: Some("rejoin".to_owned()),
                branch: Some("main".to_owned()),
                cwd: PathBuf::from("workspace/rejoin"),
                title: "Build the unified session manager".to_owned(),
                status: SessionStatus::Active,
                last_activity: Utc::now(),
                transcript: PathBuf::from("session.jsonl"),
                preview: "Implemented the session scanner and responsive interface.".to_owned(),
                archived: false,
                parse_error: None,
                preview_loaded: true,
            }],
            warnings: Vec::new(),
            scan_options: ScanOptions {
                claude_home: PathBuf::from(".claude"),
                codex_home: PathBuf::from(".codex"),
                cursor_home: PathBuf::from(".cursor"),
                pi_sessions: PathBuf::from(".pi/agent/sessions"),
                opencode_database: PathBuf::from(".local/share/opencode/opencode.db"),
                scope: Some(PathBuf::from("workspace/rejoin")),
            },
            active_agent: Agent::Claude,
            panel_selections: [0; 5],
            selected: 0,
            search: String::new(),
            filters: Filters::default(),
            draft_filters: Filters::default(),
            filter_field: 0,
            mode: Mode::Normal,
            handoff: None,
            handoff_scroll: 0,
            launch_target: None,
            toast: None,
        }
    }

    fn render(width: u16, height: u16, app: &mut App) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn wide_layout_renders_all_engine_panels() {
        let output = render(140, 30, &mut app());
        assert!(output.contains("Claude (1)"));
        assert!(!output.contains("Detail"));
        assert!(output.contains("Build the unified"));
        assert!(output.contains("Codex"));
        assert!(output.contains("OpenCode"));
        assert!(output.contains("x launch"));
    }

    #[test]
    fn compact_layout_keeps_core_actions_visible() {
        let output = render(72, 14, &mut app());
        assert!(output.contains("Claude (1)"));
        assert!(output.contains("resume"));
        assert!(output.contains("handoff"));
    }

    #[test]
    fn footer_shows_folder_without_top_status_bar() {
        let output = render(140, 30, &mut app());
        assert!(output.contains("folder: workspace/rejoin"));
        assert!(!output.contains("sessions ·"));
        assert!(!output.contains("sort:"));
    }

    #[test]
    fn handoff_overlay_is_reviewable() {
        let mut app = app();
        app.mode = Mode::Handoff;
        app.handoff = Some(Handoff {
            markdown: "# Handoff: Demo\n\n## Remaining work\n\n- Verify it.".to_owned(),
            suggested_name: "HANDOFF-demo.md".to_owned(),
        });
        let output = render(100, 28, &mut app);
        assert!(output.contains("Agent-neutral handoff"));
        assert!(output.contains("Remaining work"));
        assert!(output.contains("choose receiving agent"));
    }

    #[test]
    fn control_arrows_switch_panels_without_moving_sessions() {
        let mut app = app();
        assert_eq!(app.active_agent, Agent::Claude);

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(app.active_agent, Agent::Cursor);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(app.active_agent, Agent::Claude);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.active_agent, Agent::Claude);
    }
}
