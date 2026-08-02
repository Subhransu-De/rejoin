use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
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
    let history = load_history(&home.join("history.jsonl"));
    let projects = home.join("projects");
    let enumerate_started = Instant::now();
    let files = jsonl_files(&projects)?;
    profile("claude files", enumerate_started);
    let parse_started = Instant::now();
    let sessions = parallel_map(files, |path| {
        let mut session = match cache.get(&path) {
            Some(session) => session,
            None => match parse_session(&path, &history) {
                Ok(session) => session,
                Err(error) => error_session(path, error),
            },
        };
        if let Some(title) = history.get(&session.id) {
            session.title.clone_from(title);
        }
        session
    });
    profile("claude parse", parse_started);
    Ok(sessions)
}

fn parse_session(path: &Path, history: &HashMap<String, String>) -> Result<Session> {
    let head = head_values(path)?;
    let mut id = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    let mut cwd = None;
    let mut branch = None;
    let mut slug = None;
    let mut first_user = None;

    for value in &head {
        if let Some(found) = value.get("sessionId").and_then(Value::as_str) {
            id = found.to_owned();
        }
        cwd = cwd.or_else(|| value.get("cwd").and_then(Value::as_str).map(PathBuf::from));
        branch = branch.or_else(|| {
            value
                .get("gitBranch")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });
        slug = slug.or_else(|| {
            value
                .get("slug")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });
        if value.get("type").and_then(Value::as_str) == Some("user")
            && !value
                .get("isMeta")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            && let Some(text) = value.get("message").and_then(message_text)
            && useful_user_text(&text)
        {
            first_user.get_or_insert(text);
        }
    }

    let cwd = cwd.context("session has no working directory")?;
    let title = history
        .get(&id)
        .cloned()
        .or(slug)
        .or_else(|| first_user.map(|text| clean_text(&text, 72)))
        .unwrap_or_else(|| "Untitled Claude session".to_owned());
    let project = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(Session {
        id,
        agent: Agent::Claude,
        project,
        repository: None,
        branch,
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
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("assistant"))
        .find_map(|value| value.get("message").and_then(message_text))
        .map(|text| clean_text(&text, 320))
        .unwrap_or_default())
}

fn load_history(path: &Path) -> HashMap<String, String> {
    let Ok(file) = File::open(path) else {
        return HashMap::new();
    };
    let mut history = HashMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        let Some(display) = value.get("display").and_then(Value::as_str) else {
            continue;
        };
        if useful_user_text(display) && !display.starts_with('/') {
            history
                .entry(id.to_owned())
                .or_insert_with(|| clean_text(display, 72));
        }
    }
    history
}

fn error_session(path: PathBuf, error: anyhow::Error) -> Session {
    let id = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unreadable".to_owned());
    let cwd = path.parent().unwrap_or(Path::new("")).to_path_buf();
    Session {
        id,
        agent: Agent::Claude,
        project: "unreadable".to_owned(),
        repository: None,
        branch: None,
        cwd,
        title: "Unreadable Claude session".to_owned(),
        status: SessionStatus::Error,
        last_activity: modified_time(&path),
        transcript: path,
        preview: String::new(),
        archived: false,
        parse_error: Some(format!("{error:#}")),
        preview_loaded: true,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn parses_claude_metadata_and_preview() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("abc.jsonl");
        fs::write(
            &path,
            concat!(
                r#"{"type":"user","sessionId":"abc","cwd":"/tmp/demo","gitBranch":"main","message":{"role":"user","content":"Build the thing"}}"#,
                "\n",
                r#"{"type":"assistant","sessionId":"abc","cwd":"/tmp/demo","message":{"role":"assistant","content":[{"type":"text","text":"Implemented the parser."}]}}"#,
                "\n"
            ),
        )
        .unwrap();

        let session = parse_session(&path, &HashMap::new()).unwrap();
        assert_eq!(session.id, "abc");
        assert_eq!(session.title, "Build the thing");
        assert_eq!(load_preview(&path).unwrap(), "Implemented the parser.");
        assert_eq!(session.branch.as_deref(), Some("main"));
    }
}
