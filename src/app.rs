use ratatui::{layout::Rect, widgets::TableState};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::handoff;
use crate::launch::{LaunchKind, LaunchRequest};
use crate::model::{Agent, Filters, Handoff, Session};
use crate::scanner::{self, ScanOptions};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Search,
    Filter,
    Help,
    Warnings,
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

#[derive(Default)]
pub struct VisibleCache {
    query: String,
    filters: Filters,
    archived: bool,
    indices: [Vec<usize>; 5],
    valid: bool,
}

pub struct App {
    pub sessions: Vec<Session>,
    pub table_states: [TableState; 5],
    pub panel_areas: [Rect; 5],
    pub help_scroll: u16,
    pub show_archived: bool,
    pub visible_cache: RefCell<VisibleCache>,
    pub last_clock_refresh: Instant,
    pub scan_task: Option<Receiver<scanner::ScanResult>>,
    pub handoff_task: Option<Receiver<Result<Handoff, String>>>,
    pub pending_mode: Mode,
    pub warnings: Vec<String>,
    pub scan_options: ScanOptions,
    pub active_agent: Agent,
    pub panel_selections: [usize; 5],
    pub selected: usize,
    pub search: String,
    pub filters: Filters,
    pub draft_filters: Filters,
    pub filter_field: usize,
    pub mode: Mode,
    pub handoff: Option<Handoff>,
    pub handoff_session: Option<Session>,
    pub handoff_scroll: u16,
    pub launch_target: Option<Agent>,
    pub toast: Option<Toast>,
}

impl App {
    pub fn load(scan_options: ScanOptions) -> Self {
        let mut app = Self {
            sessions: Vec::new(),
            warnings: Vec::new(),
            table_states: std::array::from_fn(|_| TableState::default()),
            panel_areas: [Rect::default(); 5],
            help_scroll: 0,
            show_archived: false,
            visible_cache: RefCell::default(),
            last_clock_refresh: Instant::now(),
            scan_task: None,
            handoff_task: None,
            pending_mode: Mode::Normal,
            scan_options,
            active_agent: Agent::Codex,
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
        };
        app.refresh();
        app
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.visible_indices_for(self.active_agent)
    }

    pub fn visible_indices_for(&self, agent: Agent) -> Vec<usize> {
        let query = self.search.to_lowercase();
        let mut cache = self.visible_cache.borrow_mut();
        if !cache.valid
            || cache.query != query
            || cache.filters != self.filters
            || cache.archived != self.show_archived
        {
            cache.indices = std::array::from_fn(|panel| {
                let mut indices = self
                    .sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, session)| {
                        session.agent == Agent::ALL[panel]
                            && (self.show_archived || !session.archived)
                            && self.filters.matches(session)
                            && (query.is_empty() || session.search_text().contains(&query))
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                indices
                    .sort_by(|a, b| self.compare_sessions(&self.sessions[*a], &self.sessions[*b]));
                indices
            });
            cache.query = query;
            cache.filters = self.filters.clone();
            cache.archived = self.show_archived;
            cache.valid = true;
        }
        cache.indices[Agent::ALL
            .iter()
            .position(|candidate| *candidate == agent)
            .unwrap_or(0)]
        .clone()
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

    pub fn refresh(&mut self) {
        if self.scan_task.is_some() {
            return;
        }
        let options = self.scan_options.clone();
        let (sender, receiver) = mpsc::channel();
        self.scan_task = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(scanner::scan(&options));
        });
    }

    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if self.last_clock_refresh.elapsed() >= Duration::from_secs(60) {
            self.last_clock_refresh = Instant::now();
            self.visible_cache.borrow_mut().valid = false;
            self.clamp_selection();
            self.hydrate_selected_preview();
            changed = true;
        }
        if let Some(result) = self
            .scan_task
            .as_ref()
            .and_then(|task| match task.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(scanner::ScanResult {
                    sessions: Vec::new(),
                    warnings: vec!["Session scan failed".into()],
                }),
                Err(mpsc::TryRecvError::Empty) => None,
            })
        {
            let first_load = self.sessions.is_empty();
            self.sessions = result.sessions;
            self.warnings = result.warnings;
            self.scan_task = None;
            self.visible_cache.borrow_mut().valid = false;
            if first_load
                && let Some(session) = self.sessions.iter().find(|session| !session.archived)
            {
                self.active_agent = session.agent;
            }
            self.clamp_selection();
            self.hydrate_selected_preview();
            changed = true;
        }
        if let Some(result) = self
            .handoff_task
            .as_ref()
            .and_then(|task| match task.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Handoff generation failed".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            })
        {
            self.handoff_task = None;
            if self.mode == Mode::Handoff {
                match result {
                    Ok(handoff) => {
                        self.handoff = Some(handoff);
                        self.mode = self.pending_mode;
                    }
                    Err(error) => {
                        self.toast = Some(Toast::new(error, true));
                        self.mode = Mode::Normal;
                    }
                }
            }
            changed = true;
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| !toast.is_error && toast.expired())
        {
            self.toast = None;
            changed = true;
        }
        changed
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind != crossterm::event::KeyEventKind::Press {
            return AppAction::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return AppAction::Quit;
        }
        if self.toast.as_ref().is_some_and(|toast| toast.is_error) {
            self.toast = None;
        }
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Search => self.handle_search(key),
            Mode::Filter => self.handle_filter(key),
            Mode::Help | Mode::Warnings => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.mode = Mode::Normal;
                }
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.help_scroll = self.help_scroll.saturating_add(1)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.help_scroll = self.help_scroll.saturating_sub(1)
                    }
                    KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(10),
                    KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                    _ => {}
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
            KeyCode::Tab => {
                self.cycle_panel(true);
                AppAction::None
            }
            KeyCode::BackTab => {
                self.cycle_panel(false);
                AppAction::None
            }
            KeyCode::Char(character @ '1'..='5') => {
                self.focus_agent(Agent::ALL[character as usize - '1' as usize]);
                AppAction::None
            }
            KeyCode::Char('a') => {
                self.show_archived = !self.show_archived;
                self.selected = 0;
                self.hydrate_selected_preview();
                AppAction::None
            }
            KeyCode::Char('!') => {
                self.help_scroll = 0;
                self.mode = Mode::Warnings;
                AppAction::None
            }
            KeyCode::Char('q') => AppAction::Quit,
            KeyCode::Esc => {
                if !self.search.is_empty() || !self.filters.is_empty() {
                    self.search.clear();
                    self.filters = Filters::default();
                    self.selected = 0;
                }
                self.hydrate_selected_preview();
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
            KeyCode::Char('n') => self.start_new_session(),
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
                self.help_scroll = 0;
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
                self.hydrate_selected_preview();
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
            KeyCode::Char('x') if self.handoff.is_some() => {
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
                let Some(session) = self.handoff_session.as_ref() else {
                    return AppAction::None;
                };
                let Some(handoff) = &self.handoff else {
                    return AppAction::None;
                };
                let Some(target) = self.launch_target else {
                    return AppAction::None;
                };
                AppAction::Launch(LaunchRequest {
                    #[cfg(windows)]
                    cursor_home: self.scan_options.cursor_home.clone(),
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
            #[cfg(windows)]
            cursor_home: self.scan_options.cursor_home.clone(),
            kind: LaunchKind::Resume {
                agent: session.agent,
                session_id: if session.agent == Agent::Pi {
                    session.transcript.to_string_lossy().into_owned()
                } else {
                    session.id.clone()
                },
            },
            cwd: session.cwd.clone(),
        })
    }

    pub fn show_launch_error(&mut self, error: &anyhow::Error) {
        self.toast = Some(Toast::new(
            format!("Could not start agent: {error:#}"),
            true,
        ));
    }

    fn start_new_session(&self) -> AppAction {
        AppAction::Launch(LaunchRequest {
            #[cfg(windows)]
            cursor_home: self.scan_options.cursor_home.clone(),
            kind: LaunchKind::New {
                agent: self.active_agent,
            },
            cwd: self
                .scan_options
                .scope
                .clone()
                .unwrap_or_else(|| PathBuf::from(".")),
        })
    }

    fn open_handoff(&mut self, next_mode: Mode) {
        if self.handoff_task.is_some() {
            return;
        }
        let Some(session) = self.selected_session().cloned() else {
            self.toast = Some(Toast::new("No session selected", true));
            return;
        };
        self.handoff_session = Some(session.clone());
        self.handoff = None;
        self.handoff_scroll = 0;
        self.launch_target = Agent::ALL.into_iter().find(|agent| *agent != session.agent);
        self.pending_mode = next_mode;
        self.mode = Mode::Handoff;
        let (sender, receiver) = mpsc::channel();
        self.handoff_task = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(handoff::generate(&session).map_err(|error| format!("{error:#}")));
        });
    }

    fn ensure_launch_target(&mut self) {
        let Some(source) = self.handoff_session.as_ref().map(|session| session.agent) else {
            return;
        };
        if self.launch_target.is_none() || self.launch_target == Some(source) {
            self.launch_target = Agent::ALL.into_iter().find(|agent| *agent != source);
        }
    }

    fn cycle_launch_target(&mut self, forward: bool) {
        let Some(source) = self.handoff_session.as_ref().map(|session| session.agent) else {
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
        let Some(session) = self.handoff_session.as_ref() else {
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

    fn cycle_panel(&mut self, forward: bool) {
        let current = Agent::ALL
            .iter()
            .position(|agent| *agent == self.active_agent)
            .unwrap_or(0);
        self.focus_agent(Agent::ALL[(current + if forward { 1 } else { 4 }) % 5]);
    }

    fn focus_agent(&mut self, agent: Agent) {
        let current = Agent::ALL
            .iter()
            .position(|candidate| *candidate == self.active_agent)
            .unwrap_or(0);
        self.panel_selections[current] = self.selected;
        self.active_agent = agent;
        let next = Agent::ALL
            .iter()
            .position(|candidate| *candidate == agent)
            .unwrap_or(0);
        self.selected = self.panel_selections[next];
        self.clamp_selection();
        self.hydrate_selected_preview();
    }

    fn switch_agent(&mut self, direction: KeyCode) {
        let current = Agent::ALL
            .iter()
            .position(|agent| *agent == self.active_agent)
            .unwrap_or(0);
        let area = self.panel_areas[current];
        if area.is_empty() {
            self.cycle_panel(matches!(direction, KeyCode::Right | KeyCode::Down));
            return;
        }
        let center = |area: Rect| {
            (
                i32::from(area.x) * 2 + i32::from(area.width),
                i32::from(area.y) * 2 + i32::from(area.height),
            )
        };
        let (x, y) = center(area);
        let next = self
            .panel_areas
            .iter()
            .enumerate()
            .filter(|(index, area)| *index != current && !area.is_empty())
            .filter_map(|(index, area)| {
                let (xx, yy) = center(*area);
                let dx = xx - x;
                let dy = yy - y;
                let valid = match direction {
                    KeyCode::Left => dx < 0,
                    KeyCode::Right => dx > 0,
                    KeyCode::Up => dy < 0,
                    KeyCode::Down => dy > 0,
                    _ => false,
                };
                valid.then_some((
                    index,
                    if matches!(direction, KeyCode::Left | KeyCode::Right) {
                        dy.abs() * 10000 + dx.abs()
                    } else {
                        dx.abs() * 10000 + dy.abs()
                    },
                ))
            })
            .min_by_key(|(_, distance)| *distance)
            .map(|(index, _)| index);
        if let Some(next) = next {
            self.focus_agent(Agent::ALL[next]);
        }
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
        active_order.then_with(|| right.last_activity.cmp(&left.last_activity))
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
