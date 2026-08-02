use std::cmp::Ordering;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::handoff;
use crate::launch::{LaunchKind, LaunchRequest};
use crate::model::{Agent, Filters, Handoff, Session, SortOrder};
use crate::scanner::{self, ScanOptions};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Search,
    Filter,
    Help,
    Handoff,
    ConfirmLaunch,
}

#[derive(Debug)]
pub enum AppAction {
    None,
    Quit,
    Launch(LaunchRequest),
}

#[derive(Debug)]
pub struct Toast {
    pub message: String,
    pub is_error: bool,
    created: Instant,
}

impl Toast {
    fn new(message: impl Into<String>, is_error: bool) -> Self {
        Self {
            message: message.into(),
            is_error,
            created: Instant::now(),
        }
    }

    pub fn expired(&self) -> bool {
        self.created.elapsed() > Duration::from_secs(4)
    }
}

pub struct App {
    pub sessions: Vec<Session>,
    pub warnings: Vec<String>,
    pub scan_options: ScanOptions,
    pub active_agent: Agent,
    pub panel_selections: [usize; 5],
    pub selected: usize,
    pub search: String,
    pub filters: Filters,
    pub draft_filters: Filters,
    pub filter_field: usize,
    pub sort: SortOrder,
    pub mode: Mode,
    pub handoff: Option<Handoff>,
    pub handoff_scroll: u16,
    pub launch_target: Option<Agent>,
    pub toast: Option<Toast>,
}

impl App {
    pub fn load(scan_options: ScanOptions) -> Self {
        let scan = scanner::scan(&scan_options);
        let mut app = Self {
            sessions: scan.sessions,
            warnings: scan.warnings,
            scan_options,
            active_agent: Agent::Codex,
            panel_selections: [0; 5],
            selected: 0,
            search: String::new(),
            filters: Filters::default(),
            draft_filters: Filters::default(),
            filter_field: 0,
            sort: SortOrder::default(),
            mode: Mode::Normal,
            handoff: None,
            handoff_scroll: 0,
            launch_target: None,
            toast: None,
        };
        app.hydrate_selected_preview();
        app
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.visible_indices_for(self.active_agent)
    }

    pub fn visible_indices_for(&self, agent: Agent) -> Vec<usize> {
        let query = self.search.to_lowercase();
        let mut indices = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                session.agent == agent
                    && self.filters.matches(session)
                    && (query.is_empty() || session.search_text().contains(&query))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        indices.sort_by(|left, right| {
            self.compare_sessions(&self.sessions[*left], &self.sessions[*right])
        });
        indices
    }

    pub fn selection_for(&self, agent: Agent) -> usize {
        if agent == self.active_agent {
            return self.selected;
        }
        Agent::ALL
            .iter()
            .position(|candidate| *candidate == agent)
            .map(|index| self.panel_selections[index])
            .unwrap_or(0)
    }

    pub fn selected_session(&self) -> Option<&Session> {
        let indices = self.visible_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.sessions.get(*index))
    }

    pub fn agent_session_count(&self, agent: Agent) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.agent == agent)
            .count()
    }

    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.status == crate::model::SessionStatus::Active)
            .count()
    }

    pub fn refresh(&mut self) {
        let scan = scanner::scan(&self.scan_options);
        self.sessions = scan.sessions;
        self.warnings = scan.warnings;
        self.clamp_selection();
        self.hydrate_selected_preview();
        self.toast = Some(Toast::new(
            format!("Refreshed {} sessions", self.sessions.len()),
            false,
        ));
    }

    pub fn tick(&mut self) {
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind != crossterm::event::KeyEventKind::Press {
            return AppAction::None;
        }
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Search => self.handle_search(key),
            Mode::Filter => self.handle_filter(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.mode = Mode::Normal;
                }
                AppAction::None
            }
            Mode::Handoff => self.handle_handoff(key),
            Mode::ConfirmLaunch => self.handle_confirm(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Left | KeyCode::Up => {
                    self.switch_agent(key.code);
                    return AppAction::None;
                }
                KeyCode::Right | KeyCode::Down => {
                    self.switch_agent(key.code);
                    return AppAction::None;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('q') => AppAction::Quit,
            KeyCode::Esc => {
                if !self.search.is_empty() || !self.filters.is_empty() {
                    self.search.clear();
                    self.filters = Filters::default();
                    self.selected = 0;
                } else {
                    return AppAction::Quit;
                }
                AppAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                AppAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                AppAction::None
            }
            KeyCode::Home => {
                self.selected = 0;
                self.hydrate_selected_preview();
                AppAction::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible_indices().len().saturating_sub(1);
                self.hydrate_selected_preview();
                AppAction::None
            }
            KeyCode::Enter => self.resume_selected(),
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                AppAction::None
            }
            KeyCode::Char('f') => {
                self.draft_filters = self.filters.clone();
                self.filter_field = 0;
                self.mode = Mode::Filter;
                AppAction::None
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.selected = 0;
                self.hydrate_selected_preview();
                self.toast = Some(Toast::new(format!("Sort: {}", self.sort.label()), false));
                AppAction::None
            }
            KeyCode::Char('r') => {
                self.refresh();
                AppAction::None
            }
            KeyCode::Char('h') => {
                self.open_handoff(Mode::Handoff);
                AppAction::None
            }
            KeyCode::Char('x') => {
                self.open_handoff(Mode::ConfirmLaunch);
                AppAction::None
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_search(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.search.clear();
                self.mode = Mode::Normal;
                self.selected = 0;
            }
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.search.pop();
                self.selected = 0;
                self.hydrate_selected_preview();
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.search.push(character);
                self.selected = 0;
                self.hydrate_selected_preview();
            }
            _ => {}
        }
        AppAction::None
    }

    fn handle_filter(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                self.filters = self.draft_filters.clone();
                self.selected = 0;
                self.hydrate_selected_preview();
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Tab => self.filter_field = (self.filter_field + 1) % 3,
            KeyCode::Up | KeyCode::BackTab => {
                self.filter_field = self.filter_field.checked_sub(1).unwrap_or(2)
            }
            KeyCode::Left => self.cycle_filter(false),
            KeyCode::Right => self.cycle_filter(true),
            KeyCode::Backspace => match self.filter_field {
                0 => {
                    self.draft_filters.project.pop();
                }
                1 => {
                    self.draft_filters.repository.pop();
                }
                _ => {}
            },
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match self.filter_field {
                    0 => self.draft_filters.project.clear(),
                    1 => self.draft_filters.repository.clear(),
                    _ => {}
                }
            }
            KeyCode::Char(character)
                if matches!(self.filter_field, 0 | 1)
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                if self.filter_field == 0 {
                    self.draft_filters.project.push(character);
                } else {
                    self.draft_filters.repository.push(character);
                }
            }
            _ => {}
        }
        AppAction::None
    }

    fn handle_handoff(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => {
                self.handoff_scroll = self.handoff_scroll.saturating_add(1)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.handoff_scroll = self.handoff_scroll.saturating_sub(1)
            }
            KeyCode::PageDown => self.handoff_scroll = self.handoff_scroll.saturating_add(12),
            KeyCode::PageUp => self.handoff_scroll = self.handoff_scroll.saturating_sub(12),
            KeyCode::Char('c') => self.copy_handoff(),
            KeyCode::Char('w') => self.save_handoff(),
            KeyCode::Char('x') => {
                self.ensure_launch_target();
                self.mode = Mode::ConfirmLaunch;
            }
            _ => {}
        }
        AppAction::None
    }

    fn handle_confirm(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Handoff;
                AppAction::None
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.cycle_launch_target(false);
                AppAction::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.cycle_launch_target(true);
                AppAction::None
            }
            KeyCode::Enter => {
                let Some(session) = self.selected_session() else {
                    return AppAction::None;
                };
                let Some(handoff) = &self.handoff else {
                    return AppAction::None;
                };
                let Some(target) = self.launch_target else {
                    return AppAction::None;
                };
                AppAction::Launch(LaunchRequest {
                    kind: LaunchKind::Handoff {
                        target,
                        markdown: handoff.markdown.clone(),
                    },
                    cwd: session.cwd.clone(),
                })
            }
            _ => AppAction::None,
        }
    }

    fn resume_selected(&self) -> AppAction {
        let Some(session) = self.selected_session() else {
            return AppAction::None;
        };
        AppAction::Launch(LaunchRequest {
            kind: LaunchKind::Resume {
                agent: session.agent,
                session_id: session.id.clone(),
            },
            cwd: session.cwd.clone(),
        })
    }

    fn open_handoff(&mut self, next_mode: Mode) {
        let Some(session) = self.selected_session() else {
            self.toast = Some(Toast::new("No session selected", true));
            return;
        };
        let source = session.agent;
        let generated = handoff::generate(session);
        match generated {
            Ok(handoff) => {
                self.handoff = Some(handoff);
                self.handoff_scroll = 0;
                self.launch_target = Agent::ALL.into_iter().find(|agent| *agent != source);
                self.mode = next_mode;
            }
            Err(error) => {
                self.toast = Some(Toast::new(format!("{error:#}"), true));
            }
        }
    }

    fn ensure_launch_target(&mut self) {
        let Some(source) = self.selected_session().map(|session| session.agent) else {
            return;
        };
        if self.launch_target.is_none() || self.launch_target == Some(source) {
            self.launch_target = Agent::ALL.into_iter().find(|agent| *agent != source);
        }
    }

    fn cycle_launch_target(&mut self, forward: bool) {
        let Some(source) = self.selected_session().map(|session| session.agent) else {
            return;
        };
        let candidates = Agent::ALL
            .into_iter()
            .filter(|agent| *agent != source)
            .collect::<Vec<_>>();
        let current = self
            .launch_target
            .and_then(|target| candidates.iter().position(|agent| *agent == target))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % candidates.len()
        } else {
            current.checked_sub(1).unwrap_or(candidates.len() - 1)
        };
        self.launch_target = Some(candidates[next]);
    }

    fn copy_handoff(&mut self) {
        let Some(handoff) = &self.handoff else {
            return;
        };
        match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(handoff.markdown.clone()))
        {
            Ok(()) => self.toast = Some(Toast::new("Handoff copied to clipboard", false)),
            Err(error) => self.toast = Some(Toast::new(format!("Clipboard error: {error}"), true)),
        }
    }

    fn save_handoff(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let Some(handoff) = &self.handoff else {
            return;
        };
        match handoff::save(handoff, &session.cwd) {
            Ok(path) => self.toast = Some(Toast::new(format!("Saved {}", path.display()), false)),
            Err(error) => self.toast = Some(Toast::new(format!("{error:#}"), true)),
        }
    }

    fn cycle_filter(&mut self, forward: bool) {
        if self.filter_field == 2 {
            self.draft_filters.recency = if forward {
                self.draft_filters.recency.next()
            } else {
                self.draft_filters.recency.previous()
            }
        }
    }

    fn switch_agent(&mut self, direction: KeyCode) {
        let current = Agent::ALL
            .iter()
            .position(|agent| *agent == self.active_agent)
            .unwrap_or(0);
        self.panel_selections[current] = self.selected;
        // Wide layout positions:
        // Codex  Claude  Cursor
        // Pi     OpenCode
        let next = match (current, direction) {
            (0, KeyCode::Left) => 2,
            (1, KeyCode::Left) => 0,
            (2, KeyCode::Left) => 1,
            (3, KeyCode::Left) => 4,
            (4, KeyCode::Left) => 3,
            (0, KeyCode::Right) => 1,
            (1, KeyCode::Right) => 2,
            (2, KeyCode::Right) => 0,
            (3, KeyCode::Right) => 4,
            (4, KeyCode::Right) => 3,
            (0, KeyCode::Up | KeyCode::Down) => 3,
            (1, KeyCode::Up | KeyCode::Down) => 4,
            (2, KeyCode::Up | KeyCode::Down) => 4,
            (3, KeyCode::Up | KeyCode::Down) => 0,
            (4, KeyCode::Up | KeyCode::Down) => 1,
            _ => current,
        };
        self.active_agent = Agent::ALL[next];
        self.selected = self.panel_selections[next];
        self.clamp_selection();
        self.hydrate_selected_preview();
    }

    fn move_selection(&mut self, amount: isize) {
        let count = self.visible_indices().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(count.saturating_sub(1));
        let current = Agent::ALL
            .iter()
            .position(|agent| *agent == self.active_agent)
            .unwrap_or(0);
        self.panel_selections[current] = self.selected;
        self.hydrate_selected_preview();
    }

    fn clamp_selection(&mut self) {
        self.selected = self
            .selected
            .min(self.visible_indices().len().saturating_sub(1));
    }

    fn hydrate_selected_preview(&mut self) {
        let Some(index) = self.visible_indices().get(self.selected).copied() else {
            return;
        };
        if let Err(error) = scanner::load_preview(&mut self.sessions[index]) {
            self.sessions[index].preview_loaded = true;
            self.toast = Some(Toast::new(format!("Preview unavailable: {error:#}"), true));
        }
    }

    fn compare_sessions(&self, left: &Session, right: &Session) -> Ordering {
        let active_order = status_rank(left).cmp(&status_rank(right));
        match self.sort {
            SortOrder::Recent => {
                active_order.then_with(|| right.last_activity.cmp(&left.last_activity))
            }
            SortOrder::Agent => left
                .agent
                .label()
                .cmp(right.agent.label())
                .then_with(|| right.last_activity.cmp(&left.last_activity)),
            SortOrder::Project => left
                .project
                .to_lowercase()
                .cmp(&right.project.to_lowercase())
                .then_with(|| right.last_activity.cmp(&left.last_activity)),
            SortOrder::Status => {
                active_order.then_with(|| right.last_activity.cmp(&left.last_activity))
            }
        }
    }
}

fn status_rank(session: &Session) -> u8 {
    match session.status {
        crate::model::SessionStatus::Active => 0,
        crate::model::SessionStatus::Idle => 1,
        crate::model::SessionStatus::Stale => 2,
        crate::model::SessionStatus::Error => 3,
    }
}
