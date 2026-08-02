use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::model::{Agent, Session, SessionStatus};

use super::cache::SessionCache;
use super::common::{
    clean_text, head_values, message_text, modified_time, tail_values, useful_user_text,
};
use super::{jsonl_files, parallel_map, profile};

pub fn scan(home: &Path, cache: &SessionCache) -> Result<Vec<Session>> {
    let enumerate_started = Instant::now();
    let files = jsonl_files(home)?;
    profile("pi files", enumerate_started);
    let parse_started = Instant::now();
    let sessions = parallel_map(files, |path| match cache.get(&path) {
        Some(session) => session,
        None => match parse_session(&path) {
            Ok(session) => session,
            Err(error) => error_session(path, error),
        },
    });
    profile("pi parse", parse_started);
    Ok(sessions)
}

fn parse_session(path: &Path) -> Result<Session> {
    let head = head_values(path)?;
    let meta = head
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("session"))
        .context("Pi session metadata is missing")?;
    let id = meta
        .get("id")
        .and_then(Value::as_str)
        .context("Pi session id is missing")?
        .to_owned();
    let cwd = meta
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("Pi working directory is missing")?;
    let title = head
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("session_info"))
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            head.iter()
                .filter(|value| value.get("type").and_then(Value::as_str) == Some("message"))
                .filter(|value| {
                    value.pointer("/message/role").and_then(Value::as_str) == Some("user")
                })
                .filter_map(|value| value.get("message").and_then(message_text))
                .find(|text| useful_user_text(text))
        })
        .map(|text| clean_text(&text, 72))
        .unwrap_or_else(|| "Untitled Pi session".to_owned());

    Ok(Session {
        id,
        agent: Agent::Pi,
        project: project_name(&cwd),
        repository: None,
        branch: None,
        cwd,
        title,
        status: SessionStatus::Stale,
        last_activity: modified_time(path),
        transcript: path.to_path_buf(),
        preview: String::new(),
        archived: false,
        parse_error: None,
        preview_loaded: false,
    })
}

pub(crate) fn load_preview(path: &Path) -> Result<String> {
    Ok(tail_values(path)?
        .iter()
        .rev()
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("message"))
        .filter(|value| value.pointer("/message/role").and_then(Value::as_str) == Some("assistant"))
        .find_map(|value| value.get("message").and_then(message_text))
        .map(|text| clean_text(&text, 320))
        .unwrap_or_default())
}

fn project_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn error_session(path: PathBuf, error: anyhow::Error) -> Session {
    Session {
        id: path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unreadable".to_owned()),
        agent: Agent::Pi,
        project: "unreadable".to_owned(),
        repository: None,
        branch: None,
        cwd: path.parent().unwrap_or(Path::new("")).to_path_buf(),
        title: "Unreadable Pi session".to_owned(),
        status: SessionStatus::Error,
        last_activity: modified_time(&path),
        transcript: path,
        preview: String::new(),
        archived: false,
        parse_error: Some(format!("{error:#}")),
        preview_loaded: true,
    }
}
