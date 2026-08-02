mod cache;
mod claude;
mod codex;
mod common;
mod cursor;
mod opencode;
mod pi;

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Duration;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

use crate::model::{Agent, Session, SessionStatus};
use cache::SessionCache;

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub claude_home: PathBuf,
    pub codex_home: PathBuf,
    pub cursor_home: PathBuf,
    pub pi_sessions: PathBuf,
    pub opencode_database: PathBuf,
    pub scope: Option<PathBuf>,
}

impl ScanOptions {
    pub fn discover(
        claude_home: Option<PathBuf>,
        codex_home: Option<PathBuf>,
        cursor_home: Option<PathBuf>,
        pi_session_dir: Option<PathBuf>,
        opencode_database: Option<PathBuf>,
        all: bool,
    ) -> Result<Self> {
        let home = dirs::home_dir().context("could not determine the home directory")?;
        let claude_home = claude_home
            .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
            .unwrap_or_else(|| home.join(".claude"));
        let codex_home = codex_home
            .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
            .unwrap_or_else(|| home.join(".codex"));
        let cursor_home = cursor_home
            .unwrap_or_else(|| discover_cursor_home(&home).unwrap_or_else(|| home.join(".cursor")));
        let pi_sessions = pi_session_dir
            .or_else(|| std::env::var_os("PI_CODING_AGENT_SESSION_DIR").map(PathBuf::from))
            .unwrap_or_else(|| discover_pi_sessions(&home));
        let opencode_database =
            opencode_database.unwrap_or_else(|| discover_opencode_database(&home));
        Ok(Self {
            claude_home,
            codex_home,
            cursor_home,
            pi_sessions,
            opencode_database,
            scope: if all {
                None
            } else {
                Some(std::env::current_dir().context("could not determine current directory")?)
            },
        })
    }
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub sessions: Vec<Session>,
    pub warnings: Vec<String>,
}

pub fn scan(options: &ScanOptions) -> ScanResult {
    let started = Instant::now();
    let mut result = ScanResult::default();
    let cache = SessionCache::load();

    let claude_started = Instant::now();
    match claude::scan(&options.claude_home, &cache) {
        Ok(mut sessions) => result.sessions.append(&mut sessions),
        Err(error) => result.warnings.push(format!("Claude: {error:#}")),
    }
    profile("claude", claude_started);

    let codex_started = Instant::now();
    match codex::scan(&options.codex_home, &cache) {
        Ok(mut sessions) => result.sessions.append(&mut sessions),
        Err(error) => result.warnings.push(format!("Codex: {error:#}")),
    }
    profile("codex", codex_started);

    let cursor_started = Instant::now();
    match cursor::scan(&options.cursor_home, options.scope.as_deref()) {
        Ok(mut sessions) => result.sessions.append(&mut sessions),
        Err(error) => result.warnings.push(format!("Cursor: {error:#}")),
    }
    profile("cursor", cursor_started);

    let pi_started = Instant::now();
    match pi::scan(&options.pi_sessions, &cache) {
        Ok(mut sessions) => result.sessions.append(&mut sessions),
        Err(error) => result.warnings.push(format!("Pi: {error:#}")),
    }
    profile("pi", pi_started);

    let opencode_started = Instant::now();
    match opencode::scan(&options.opencode_database) {
        Ok(mut sessions) => result.sessions.append(&mut sessions),
        Err(error) => result.warnings.push(format!("OpenCode: {error:#}")),
    }
    profile("opencode", opencode_started);

    if let Err(error) = cache.save_if_dirty(&result.sessions) {
        result.warnings.push(format!("Cache: {error:#}"));
    }

    if let Some(scope) = &options.scope {
        let scope = normalize_path(scope);
        result
            .sessions
            .retain(|session| normalize_path(&session.cwd) == scope);
    }

    let repository_started = Instant::now();
    enrich_repository_metadata(&mut result.sessions);
    profile("repositories", repository_started);

    let process_started = Instant::now();
    apply_process_status(&mut result.sessions);
    profile("processes", process_started);

    result
        .sessions
        .sort_by_key(|session| Reverse(session.last_activity));
    profile("total", started);
    result
}

pub fn load_preview(session: &mut Session) -> Result<()> {
    if session.preview_loaded {
        return Ok(());
    }
    session.preview = match session.agent {
        Agent::Claude => claude::load_preview(&session.transcript)?,
        Agent::Codex => codex::load_preview(&session.transcript)?,
        Agent::Cursor => cursor::load_preview(&session.transcript)?,
        Agent::Pi => pi::load_preview(&session.transcript)?,
        Agent::OpenCode => opencode::load_preview(&session.transcript, &session.id)?,
    };
    session.preview_loaded = true;
    Ok(())
}

pub(crate) fn profile(stage: &str, started: Instant) {
    if std::env::var_os("REJOIN_PROFILE").is_some() {
        eprintln!(
            "rejoin profile: {stage:<14} {:>8.2} ms",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
}

fn enrich_repository_metadata(sessions: &mut [Session]) {
    let mut cache: HashMap<PathBuf, (Option<String>, Option<PathBuf>)> = HashMap::new();
    for session in sessions {
        let (repository, root) = cache
            .entry(session.cwd.clone())
            .or_insert_with(|| discover_repository(&session.cwd));
        if session.repository.is_none() {
            session.repository.clone_from(repository);
        }
        if session.project.is_empty() {
            session.project = root
                .as_deref()
                .and_then(Path::file_name)
                .or_else(|| session.cwd.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_owned());
        }
    }
}

fn discover_repository(cwd: &Path) -> (Option<String>, Option<PathBuf>) {
    let mut current = Some(cwd);
    while let Some(path) = current {
        if path.join(".git").exists() {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            return (name, Some(path.to_path_buf()));
        }
        current = path.parent();
    }
    (None, None)
}

fn apply_process_status(sessions: &mut [Session]) {
    let process_list = ProcessRefreshKind::nothing().without_tasks();
    let mut system =
        System::new_with_specifics(RefreshKind::nothing().with_processes(process_list));
    let agent_pids = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy().to_lowercase();
            (name.contains("claude")
                || name.contains("codex")
                || name.contains("cursor")
                || name.contains("opencode")
                || name == "pi"
                || name == "pi.exe"
                || name.contains("wsl"))
            .then_some(*pid)
        })
        .collect::<Vec<_>>();
    if !agent_pids.is_empty() {
        let details = ProcessRefreshKind::nothing()
            .without_tasks()
            .with_cwd(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet);
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&agent_pids), false, details);
    }
    let mut exact_ids = HashSet::new();
    let mut live_workspaces: HashSet<(Agent, String)> = HashSet::new();

    for process in system.processes().values() {
        let name = process.name().to_string_lossy().to_lowercase();
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let agent = if name.contains("claude") {
            Some(Agent::Claude)
        } else if name.contains("codex") {
            Some(Agent::Codex)
        } else if name.contains("cursor") || command.to_lowercase().contains("cursor-agent") {
            Some(Agent::Cursor)
        } else if name == "pi" || name == "pi.exe" {
            Some(Agent::Pi)
        } else if name.contains("opencode") {
            Some(Agent::OpenCode)
        } else {
            None
        };

        if let Some(agent) = agent {
            if std::env::var_os("REJOIN_PROFILE").is_some() {
                eprintln!(
                    "rejoin profile: process        {:<22} cwd={}",
                    name,
                    process
                        .cwd()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<unavailable>".to_owned())
                );
            }
            for session in sessions.iter() {
                if command.contains(&session.id) {
                    exact_ids.insert(session.id.clone());
                }
            }
            if let Some(cwd) = process.cwd() {
                live_workspaces.insert((agent, normalize_path(cwd)));
            }
        }
    }

    let mut newest_by_workspace: HashMap<(Agent, String), usize> = HashMap::new();
    for (index, session) in sessions.iter().enumerate() {
        let key = (session.agent, normalize_path(&session.cwd));
        if live_workspaces.contains(&key) {
            newest_by_workspace
                .entry(key)
                .and_modify(|current| {
                    if sessions[*current].last_activity < session.last_activity {
                        *current = index;
                    }
                })
                .or_insert(index);
        }
    }
    let inferred_active: HashSet<usize> = newest_by_workspace.into_values().collect();

    for (index, session) in sessions.iter_mut().enumerate() {
        if session.parse_error.is_some() {
            session.status = SessionStatus::Error;
        } else if exact_ids.contains(&session.id) || inferred_active.contains(&index) {
            session.status = SessionStatus::Active;
        } else if chrono::Utc::now() - session.last_activity <= Duration::days(1) {
            session.status = SessionStatus::Idle;
        } else {
            session.status = SessionStatus::Stale;
        }
    }
}

pub(crate) fn normalize_path(path: &Path) -> String {
    let normalized = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalized
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .trim_end_matches(['/', '\\'])
        .to_lowercase()
}

pub(crate) fn opencode_text_history(
    database: &Path,
    session_id: &str,
) -> Result<Vec<(String, String)>> {
    opencode::text_history(database, session_id)
}

pub(crate) fn opencode_tool_history(
    database: &Path,
    session_id: &str,
) -> Result<Vec<(String, String)>> {
    opencode::tool_history(database, session_id)
}

fn discover_pi_sessions(home: &Path) -> PathBuf {
    let agent_home = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".pi").join("agent"));
    let settings = agent_home.join("settings.json");
    let configured = std::fs::read(&settings)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("sessionDir")?.as_str().map(ToOwned::to_owned));
    match configured.map(expand_home) {
        Some(path) if path.is_absolute() => path,
        Some(path) => agent_home.join(path),
        None => agent_home.join("sessions"),
    }
}

fn expand_home(path: String) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix(r"~\")) {
        return dirs::home_dir()
            .unwrap_or_default()
            .join(rest.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR));
    }
    PathBuf::from(path)
}

fn discover_opencode_database(home: &Path) -> PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    let directory = data_home.join("opencode");
    let preferred = directory.join("opencode.db");
    std::fs::read_dir(&directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name == "opencode.db"
                        || (name.starts_with("opencode-") && name.ends_with(".db"))
                })
        })
        .max_by_key(|path| path.metadata().and_then(|meta| meta.modified()).ok())
        .unwrap_or(preferred)
}

fn discover_cursor_home(home: &Path) -> Option<PathBuf> {
    let native = home.join(".cursor");
    if native.join("chats").exists() {
        return Some(native);
    }
    if let Some(wrapper_home) = cursor_home_from_wrapper() {
        return Some(wrapper_home);
    }
    discover_wsl_cursor_home()
}

#[cfg(not(windows))]
fn cursor_home_from_wrapper() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn cursor_home_from_wrapper() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let wrapper = directory.join("cursor-agent.ps1");
        let Ok(contents) = std::fs::read_to_string(wrapper) else {
            continue;
        };
        let tokens = contents
            .lines()
            .find(|line| {
                line.contains("wsl.exe") && line.contains(" -d ") && line.contains(" -u ")
            })?
            .split_whitespace()
            .collect::<Vec<_>>();
        let distro = tokens
            .iter()
            .position(|token| *token == "-d")
            .and_then(|index| tokens.get(index + 1))?
            .trim_matches(['\'', '"']);
        let user = tokens
            .iter()
            .position(|token| *token == "-u")
            .and_then(|index| tokens.get(index + 1))?
            .trim_matches(['\'', '"']);
        let candidate = PathBuf::from(format!(r"\\wsl.localhost\{distro}\home\{user}\.cursor"));
        return Some(candidate);
    }
    None
}

#[cfg(not(windows))]
fn discover_wsl_cursor_home() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn discover_wsl_cursor_home() -> Option<PathBuf> {
    for share in [r"\\wsl$\", r"\\wsl.localhost\"] {
        for distro in std::fs::read_dir(share)
            .ok()?
            .filter_map(|entry| entry.ok())
        {
            let home = distro.path().join("home");
            for user in std::fs::read_dir(home)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok())
            {
                let candidate = user.path().join(".cursor");
                if candidate.join("chats").exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub(crate) fn jsonl_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    visit(root, &mut files)?;
    Ok(files)
}

pub(crate) fn parallel_map<T, F>(files: Vec<PathBuf>, operation: F) -> Vec<T>
where
    T: Send,
    F: Fn(PathBuf) -> T + Sync,
{
    if files.len() < 8 {
        return files.into_iter().map(operation).collect();
    }
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8)
        .min(files.len());
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(files.len()));
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let operation = &operation;
            let files = &files;
            let next = &next;
            let results = &results;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else {
                        break;
                    };
                    let value = operation(path.clone());
                    results
                        .lock()
                        .expect("scan result mutex poisoned")
                        .push((index, value));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("scan result mutex poisoned");
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, value)| value).collect()
}

fn visit(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("could not read {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "subagents") {
                continue;
            }
            visit(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_extended_prefix_and_case() {
        assert_eq!(
            normalize_path(Path::new(r"\\?\X:\Fixture\Project\")),
            normalize_path(Path::new(r"x:\fixture\project"))
        );
    }
}
