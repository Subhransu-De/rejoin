use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Agent, Session, SessionStatus};

use super::cache::SessionCache;
use super::common::{
    clean_text, first_json, head_values, message_text, modified_time, tail_values, useful_user_text,
};
use super::{jsonl_files, parallel_map, profile};

pub fn scan(home: &Path, cache: &SessionCache) -> Result<Vec<Session>> {
    let mut titles = load_titles(&home.join("session_index.jsonl"));
    load_history_titles(&home.join("history.jsonl"), &mut titles);
    let mut sessions = Vec::new();
    for (root, archived) in [
        (home.join("sessions"), false),
        (home.join("archived_sessions"), true),
    ] {
        let enumerate_started = Instant::now();
        let files = jsonl_files(&root)?;
        profile("codex files", enumerate_started);
        let parse_started = Instant::now();
        let mut found = parallel_map(files, |path| {
            let mut session = match cache.get(&path) {
                Some(session) => session,
                None => match parse_session(&path, &titles, archived) {
                    Ok(session) => session,
                    Err(error) => error_session(path, error, archived),
                },
            };
            if let Some(title) = titles.get(&session.id) {
                session.title.clone_from(title);
            }
            session.archived = archived;
            session
        });
        profile("codex parse", parse_started);
        sessions.append(&mut found);
    }
    Ok(sessions)
}

fn parse_session(path: &Path, titles: &HashMap<String, String>, archived: bool) -> Result<Session> {
    let header: CodexHeader = first_json(path)?;
    let meta = header.payload.as_deref().unwrap_or(&header);
    let id = meta
        .id
        .as_deref()
        .or(meta.session_id.as_deref())
        .context("session metadata has no id")?
        .to_owned();
    let cwd = meta
        .cwd
        .clone()
        .or_else(dirs::home_dir)
        .context("session metadata has no working directory and home is unavailable")?;
    let branch = meta.git.as_ref().and_then(|git| git.branch.clone());
    let legacy_repository = meta
        .git
        .as_ref()
        .and_then(|git| git.repository_url.as_deref())
        .and_then(repository_name);

    let title = titles
        .get(&id)
        .cloned()
        .or_else(|| {
            head_values(path)
                .ok()?
                .iter()
                .find_map(codex_user_message)
                .map(|text| clean_text(&text, 72))
        })
        .unwrap_or_else(|| "Untitled Codex session".to_owned());
    let project = legacy_repository.clone().unwrap_or_else(|| {
        cwd.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    Ok(Session {
        id,
        agent: Agent::Codex,
        project,
        repository: legacy_repository,
        branch,
        cwd,
        title,
        status: SessionStatus::Stale,
        last_activity: modified_time(path),
        transcript: path.to_path_buf(),
        preview: String::new(),
        archived,
        parse_error: None,
        preview_loaded: false,
    })
}

#[derive(Debug, Default, Deserialize)]
struct CodexHeader {
    #[serde(default)]
    payload: Option<Box<CodexHeader>>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    git: Option<CodexGit>,
}

#[derive(Debug, Deserialize)]
struct CodexGit {
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    repository_url: Option<String>,
}

pub(crate) fn load_preview(path: &Path) -> Result<String> {
    Ok(tail_values(path)?
        .iter()
        .rev()
        .find_map(codex_assistant_message)
        .map(|text| clean_text(&text, 320))
        .unwrap_or_default())
}

fn codex_user_message(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("message")
        && value.get("role").and_then(Value::as_str) == Some("user")
    {
        let text = message_text(value)?;
        return useful_user_text(&text).then_some(text);
    }
    let payload = value.get("payload")?;
    if value.get("type").and_then(Value::as_str) == Some("event_msg")
        && payload.get("type").and_then(Value::as_str) == Some("user_message")
    {
        let text = payload.get("message").and_then(Value::as_str)?;
        return useful_user_text(text).then(|| text.to_owned());
    }
    None
}

fn codex_assistant_message(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("message")
        && value.get("role").and_then(Value::as_str) == Some("assistant")
    {
        return message_text(value);
    }
    let payload = value.get("payload")?;
    if value.get("type").and_then(Value::as_str) == Some("event_msg")
        && payload.get("type").and_then(Value::as_str) == Some("agent_message")
    {
        return payload
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    if value.get("type").and_then(Value::as_str) == Some("response_item")
        && payload.get("type").and_then(Value::as_str) == Some("message")
        && payload.get("role").and_then(Value::as_str) == Some("assistant")
    {
        return message_text(payload);
    }
    None
}

fn load_titles(path: &Path) -> HashMap<String, String> {
    let Ok(file) = File::open(path) else {
        return HashMap::new();
    };
    let mut titles = HashMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let title = value
            .get("thread_name")
            .or_else(|| value.get("title"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty());
        if let Some(title) = title {
            titles.insert(id.to_owned(), clean_text(title, 72));
        }
    }
    titles
}

fn load_history_titles(path: &Path, titles: &mut HashMap<String, String>) {
    let Ok(file) = File::open(path) else {
        return;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value
            .get("session_id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(text) = value
            .get("text")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if useful_user_text(text) {
            titles
                .entry(id.to_owned())
                .or_insert_with(|| clean_text(text, 72));
        }
    }
}

fn repository_name(url: &str) -> Option<String> {
    let path = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = path.strip_suffix(".git").unwrap_or(path).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn error_session(path: PathBuf, error: anyhow::Error, archived: bool) -> Session {
    let id = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unreadable".to_owned());
    let cwd = path.parent().unwrap_or(Path::new("")).to_path_buf();
    Session {
        id,
        agent: Agent::Codex,
        project: "unreadable".to_owned(),
        repository: None,
        branch: None,
        cwd,
        title: "Unreadable Codex session".to_owned(),
        status: SessionStatus::Error,
        last_activity: modified_time(&path),
        transcript: path,
        preview: String::new(),
        archived,
        parse_error: Some(format!("{error:#}")),
        preview_loaded: true,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn parses_codex_metadata_and_messages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rollout-abc.jsonl");
        fs::write(
            &path,
            concat!(
                r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"abc","cwd":"/tmp/demo"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"Fix the scanner"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"The scanner is fixed."}}"#,
                "\n"
            ),
        )
        .unwrap();

        let session = parse_session(&path, &HashMap::new(), false).unwrap();
        assert_eq!(session.id, "abc");
        assert_eq!(session.title, "Fix the scanner");
        assert_eq!(load_preview(&path).unwrap(), "The scanner is fixed.");
    }
}
