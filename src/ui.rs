use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, Wrap,
};

use crate::app::{App, Mode};
use crate::model::{Agent, SessionStatus, relative_time, truncate_width};

const CLAUDE: Color = Color::Rgb(232, 142, 79);
const CODEX: Color = Color::Rgb(55, 190, 184);
const CURSOR: Color = Color::Rgb(139, 126, 255);
const PI: Color = Color::Rgb(244, 202, 90);
const OPENCODE: Color = Color::Rgb(95, 205, 125);
const MUTED: Color = Color::Rgb(120, 128, 140);
const ACCENT: Color = Color::Rgb(122, 162, 247);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let command_height = if app.mode == Mode::Search { 3 } else { 1 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(if area.height >= 16 { 3 } else { 0 }),
            Constraint::Length(command_height),
        ])
        .split(area);

    draw_body(frame, app, rows[0]);
    draw_detail(frame, app, rows[1]);
    if app.mode == Mode::Search {
        draw_search(frame, app, rows[2]);
    } else {
        draw_footer(frame, app, rows[2]);
    }

    match app.mode {
        Mode::Search => {}
        Mode::Filter => draw_filter(frame, app),
        Mode::Help => draw_help(frame, app),
        Mode::Warnings => draw_warnings(frame, app),
        Mode::Handoff => draw_handoff(frame, app),
        Mode::ConfirmLaunch => draw_confirm(frame, app),
        Mode::Normal => {}
    }
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let tabs = Agent::ALL
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            Span::styled(
                format!(" {}:{} ", i + 1, agent.label()),
                if *agent == app.active_agent {
                    agent_style(*agent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(MUTED)
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Line::from(tabs)), rows[0]);
    app.panel_areas = [Rect::default(); 5];
    let agents = if area.width < 100 || area.height < 22 {
        vec![app.active_agent]
    } else {
        Agent::ALL
            .into_iter()
            .filter(|agent| {
                *agent == app.active_agent || !app.visible_indices_for(*agent).is_empty()
            })
            .collect::<Vec<_>>()
    };
    if agents.len() <= 3 {
        let columns = Layout::horizontal(vec![Constraint::Fill(1); agents.len()]).split(rows[1]);
        for (agent, area) in agents.iter().zip(columns.iter()) {
            draw_sessions(frame, app, *agent, *area);
        }
    } else {
        let panels = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).split(rows[1]);
        let split = agents.len().div_ceil(2);
        for (row, group) in [
            (&panels[0], &agents[..split]),
            (&panels[1], &agents[split..]),
        ] {
            let columns = Layout::horizontal(vec![Constraint::Fill(1); group.len()]).split(*row);
            for (agent, area) in group.iter().zip(columns.iter()) {
                draw_sessions(frame, app, *agent, *area);
            }
        }
    }
}

fn draw_sessions(frame: &mut Frame, app: &mut App, agent: Agent, area: Rect) {
    let panel = Agent::ALL
        .iter()
        .position(|candidate| *candidate == agent)
        .unwrap_or(0);
    app.panel_areas[panel] = area;
    let indices = app.visible_indices_for(agent);
    let focused = agent == app.active_agent;
    let border_style = if focused {
        agent_style(agent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let title = Line::from(vec![
        Span::raw(format!(
            " {}{} ({})  ",
            if focused { "● " } else { "" },
            agent.label(),
            indices.len()
        )),
        Span::styled(
            "[n - New]",
            if focused {
                agent_style(agent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ),
        Span::raw(" "),
    ]);
    if indices.is_empty() {
        let message = if app.scan_task.is_some() {
            "Scanning sessions…"
        } else if !app.warnings.is_empty() {
            "Some stores could not be read. Press ! for details."
        } else if app.agent_session_count(agent) == 0 {
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
                        .title(title.clone()),
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
            Cell::from(truncate_width(
                &format!(
                    "{}{}",
                    if session.archived { "[archived] " } else { "" },
                    session.title
                ),
                usize::from(area.width.saturating_sub(18)),
            )),
            Cell::from(
                Line::from(relative_time(session.last_activity)).alignment(Alignment::Center),
            ),
        ])
    });
    let widths = [
        Constraint::Length(2),
        Constraint::Min(16),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec![
                Cell::from(""),
                Cell::from("Session"),
                Cell::from(Line::from("Activity").alignment(Alignment::Center)),
            ])
            .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD)),
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
    app.table_states[panel].select(Some(app.selection_for(agent)));
    frame.render_stateful_widget(table, area, &mut app.table_states[panel]);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(toast) = &app.toast {
        frame.render_widget(
            Paragraph::new(truncate_width(&toast.message, usize::from(area.width))).style(
                Style::default().fg(if toast.is_error {
                    Color::Red
                } else {
                    Color::Green
                }),
            ),
            area,
        );
        return;
    }
    let full_folder = app
        .scan_options
        .scope
        .as_ref()
        .map(|scope| format!(" {} ", scope.display()))
        .unwrap_or_else(|| " all folders ".to_owned());
    let max_folder_width = if app.mode == Mode::Normal {
        (area.width.saturating_mul(45) / 100).min(area.width.saturating_sub(44))
    } else {
        0
    };
    let folder = truncate_left(&full_folder, usize::from(max_folder_width));
    let folder_width = unicode_width::UnicodeWidthStr::width(folder.as_str()) as u16;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(folder_width)])
        .split(area);
    let content = match app.mode {
        Mode::Normal if columns[0].width < 100 => " ? help  q quit  n new  ↵ resume ",
        Mode::Normal => {
            " n new   ↑/↓ session   Tab panel   ↵ resume   h handoff   x launch   / search   f filter   ? help   q quit "
        }
        Mode::Search => " type to search   ↵ keep   Esc clear ",
        Mode::Filter => " ↑↓ field   ←→ choose   type values   ↵ apply   Esc cancel ",
        Mode::Help | Mode::Warnings => " ↑↓ scroll   Esc close ",
        Mode::Handoff => " ↑↓ scroll  c copy  w save  x choose agent  Esc close ",
        Mode::ConfirmLaunch => " ←→ choose agent   ↵ launch with handoff   Esc review ",
    };
    let footer_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(180, 190, 210));
    frame.render_widget(Block::default().style(footer_style), area);
    let content = if !app.warnings.is_empty() {
        format!(" !{} {content}", app.warnings.len())
    } else if app.scan_task.is_some() {
        format!(" Scanning…  {content}")
    } else {
        content.to_owned()
    };
    frame.render_widget(Paragraph::new(content).style(footer_style), columns[0]);
    frame.render_widget(
        Paragraph::new(folder)
            .alignment(Alignment::Right)
            .style(footer_style),
        columns[1],
    );
}

fn truncate_left(value: &str, max_width: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if value.width() <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut tail = Vec::new();
    for character in value.chars().rev() {
        let width = character.width().unwrap_or(0);
        if used + width > max_width {
            break;
        }
        tail.push(character);
        used += width;
    }
    format!("…{}", tail.into_iter().rev().collect::<String>())
}

fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(" Search sessions ");
    let input_area = block.inner(area);
    frame.render_widget(block, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(28)])
        .split(input_area);
    let query = if app.search.is_empty() {
        Line::from(vec![
            Span::styled(" / ", Style::default().fg(ACCENT)),
            Span::styled(
                "type to filter sessions",
                Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
            ),
            Span::styled("█", Style::default().fg(ACCENT)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" / ", Style::default().fg(ACCENT)),
            Span::styled(
                truncate_left(&app.search, usize::from(columns[0].width.saturating_sub(4))),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(ACCENT)),
        ])
    };
    frame.render_widget(Paragraph::new(query), columns[0]);
    frame.render_widget(
        Paragraph::new("Enter keep   Esc clear ")
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED)),
        columns[1],
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

fn draw_help(frame: &mut Frame, app: &App) {
    let area = centered(
        frame.area(),
        90,
        26.min(frame.area().height.saturating_sub(2)),
    );
    frame.render_widget(Clear, area);
    let help = "\
Navigation\n\
  Tab/Shift+Tab or 1–5: panels; Ctrl+arrows: adjacent panel      ↑/k, ↓/j     select session\n\
  Home, G      first / last session\n\n\
Actions\n\
  n             start a new session   Enter         resume in its agent\n\
  x             cross-launch agent    h             preview handoff\n\
  r             rescan local stores\n\n\
Find\n\
  /             search sessions    f             structured filters\n\
  Esc           clear search/filters    q / Ctrl+C    quit\n\
  a             show archived sessions  !             scanner warnings\n\n\
Handoff\n\
  c             copy Markdown         w             save outside the repository\n\
  x             choose any other agent for the reviewed package\n\n\
Scope\n\
  By default only the exact current folder is shown. Use rejoin --all for all folders.\n\n\
Status: ● active (session id matched), ◐ recent (<24h), ○ stale, × error.\n\
Workspace matches alone do not prove that a session is running.";
    frame.render_widget(
        Paragraph::new(help)
            .scroll((app.help_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .title(" Help · ↑↓ scroll · Esc close "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_handoff(frame: &mut Frame, app: &App) {
    let area = if frame.area().width < 120 {
        frame.area().inner(Margin::new(2, 1))
    } else {
        centered(
            frame.area(),
            90,
            frame.area().height.saturating_mul(84) / 100,
        )
    };
    frame.render_widget(Clear, area);
    let markdown = app
        .handoff
        .as_ref()
        .map(|handoff| handoff.markdown.as_str())
        .unwrap_or(if app.handoff_task.is_some() {
            "Building handoff…"
        } else {
            "Handoff unavailable."
        });
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
    let Some(session) = app.handoff_session.as_ref() else {
        return;
    };
    let Some(target) = app.launch_target else {
        return;
    };
    let area = centered(frame.area(), 90, 9);
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

fn centered(area: Rect, width_percent: u16, height: u16) -> Rect {
    let width = area.width.saturating_mul(width_percent).saturating_div(100);
    let height = height.min(area.height);
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

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(session) = app.selected_session() else {
        return;
    };
    let details = vec![
        Line::from(truncate_width(&session.title, usize::from(area.width))),
        Line::from(truncate_width(
            &format!(
                "{} · {} · {} · {} · {}",
                session.status.label(),
                session.branch.as_deref().unwrap_or("no branch"),
                session.last_activity.format("%Y-%m-%d %H:%M UTC"),
                session.id,
                session.cwd.display()
            ),
            usize::from(area.width),
        )),
        Line::from(truncate_width(&session.preview, usize::from(area.width))),
    ];
    frame.render_widget(Paragraph::new(details), area);
}

fn draw_warnings(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 95, frame.area().height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let text = if app.warnings.is_empty() {
        "No scanner warnings.".to_owned()
    } else {
        app.warnings.join("\n\n")
    };
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((app.help_scroll, 0))
            .block(Block::bordered().title(" Scanner warnings · ↑↓ scroll · Esc close ")),
        area,
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::{AppAction, Mode};
    use crate::launch::LaunchKind;
    use crate::model::{Filters, Handoff};
    use crate::scanner::ScanOptions;
    use ratatui::widgets::TableState;

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
            table_states: std::array::from_fn(|_| TableState::default()),
            panel_areas: [Rect::default(); 5],
            help_scroll: 0,
            show_archived: false,
            visible_cache: std::cell::RefCell::default(),
            scan_task: None,
            handoff_task: None,
            pending_mode: Mode::Normal,
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
            handoff_session: None,
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
        assert!(output.contains("[n - New]"));
        assert!(output.contains("n new"));
        assert!(output.contains("x launch"));
    }

    #[test]
    fn compact_layout_keeps_core_actions_visible() {
        let output = render(72, 14, &mut app());
        assert!(output.contains("Claude (1)"));
        assert!(output.contains("[n - New]"));
        assert!(output.contains("resume"));
    }

    #[test]
    fn footer_shows_folder_without_top_status_bar() {
        let output = render(140, 30, &mut app());
        assert!(output.contains("workspace/rejoin"));
        assert!(!output.contains("folder:"));
        assert!(!output.contains("sessions ·"));
        assert!(!output.contains("sort:"));
    }

    #[test]
    fn search_mode_shows_the_query_in_a_dedicated_input_bar() {
        let mut app = app();
        app.mode = Mode::Search;
        app.search = "cursor".to_owned();

        let output = render(140, 30, &mut app);

        assert!(output.contains("Search sessions"));
        assert!(output.contains("/ cursor█"));
        assert!(output.contains("Enter keep"));
        assert!(output.contains("Esc clear"));
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
        assert!(output.contains("choose agent"));
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

    #[test]
    fn new_session_action_uses_the_focused_agent_and_current_folder() {
        let mut app = app();

        let action = app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        let AppAction::Launch(request) = action else {
            panic!("n should launch a new session");
        };
        let LaunchKind::New { agent } = request.kind else {
            panic!("n should not resume an existing session");
        };
        assert_eq!(agent, Agent::Claude);
        assert_eq!(request.cwd, PathBuf::from("workspace/rejoin"));
    }

    #[test]
    fn launch_errors_are_shown_inside_the_session_manager() {
        let mut app = app();

        app.show_launch_error(&anyhow::anyhow!("program not found"));

        let toast = app.toast.expect("launch failure should be visible");
        assert!(toast.is_error);
        assert!(toast.message.contains("program not found"));
    }

    #[test]
    fn overlays_have_readable_content_at_supported_sizes() {
        for (width, height) in [(60, 12), (80, 24), (120, 40), (160, 42)] {
            let mut app = app();
            app.mode = Mode::ConfirmLaunch;
            app.launch_target = Some(Agent::Codex);
            app.handoff_session = Some(app.sessions[0].clone());
            let output = render(width, height, &mut app);
            assert!(output.contains("Launch Codex in:"));
            app.mode = Mode::Filter;
            assert!(render(width, height, &mut app).contains("Recency"));
            app.mode = Mode::Help;
            assert!(render(width, height, &mut app).contains("Navigation"));
        }
    }

    #[test]
    fn upward_navigation_keeps_the_scroll_offset() {
        let mut app = app();
        let session = app.sessions[0].clone();
        app.sessions = (0..40)
            .map(|index| {
                let mut item = session.clone();
                item.id = format!("synthetic-{index}");
                item.title = format!("Synthetic session {index:02}");
                item
            })
            .collect();
        app.selected = 30;
        render(80, 24, &mut app);
        let offset = app.table_states[1].offset();
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        render(80, 24, &mut app);
        assert_eq!(app.table_states[1].offset(), offset);
        assert_eq!(app.table_states[1].selected(), Some(29));
    }

    #[test]
    fn search_is_independent_of_preview_hydration() {
        let mut app = app();
        app.search = "synthetic-preview-only".into();
        assert!(app.visible_indices().is_empty());
        app.sessions[0].preview = "synthetic-preview-only".into();
        assert!(app.visible_indices().is_empty());
        app.search = "session-1".into();
        assert_eq!(app.visible_indices(), vec![0]);
    }

    #[test]
    fn escape_clears_and_control_c_quits_in_every_mode() {
        let mut app = app();
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            AppAction::None
        ));
        for mode in [
            Mode::Normal,
            Mode::Help,
            Mode::Search,
            Mode::Filter,
            Mode::Handoff,
            Mode::ConfirmLaunch,
            Mode::Warnings,
        ] {
            app.mode = mode;
            assert!(matches!(
                app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                AppAction::Quit
            ));
        }
    }

    #[test]
    fn warnings_and_compact_help_remain_visible() {
        let mut app = app();
        app.warnings.push("Synthetic unreadable store".into());
        app.mode = Mode::Warnings;
        assert!(render(80, 24, &mut app).contains("Synthetic unreadable store"));
        app.mode = Mode::Normal;
        assert!(render(60, 12, &mut app).contains("? help"));
        assert!(render(60, 12, &mut app).contains("q quit"));
        app.mode = Mode::Handoff;
        assert!(render(60, 12, &mut app).contains("Esc close"));
    }
}
