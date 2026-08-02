use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Agent, Session, SessionStatus};

use super::common::{clean_text, head_values, message_text, tail_values, useful_user_text};
use super::{jsonl_files, parallel_map};

pub fn scan(home: &Path, scope: Option<&Path>) -> Result<Vec<Session>> {
    if !home.join("chats").exists() {
        return Ok(Vec::new());
    }
    let mut transcripts = None;
    let mut sessions = Vec::new();
    let metadata = parallel_map(named_files(&home.join("chats"), "meta.json")?, |path| {
        let id = path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned());
        let meta = std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| serde_json::from_slice::<CursorMeta>(&bytes).map_err(Into::into));
        (path, id, meta)
    });
    for (path, id, meta) in metadata {
        let Some(id) = id else {
            continue;
        };
        let meta = meta?;
        if !meta.has_conversation {
            continue;
        }
        let cwd = wsl_to_windows(&meta.cwd);
        if scope.is_some_and(|scope| super::normalize_path(scope) != super::normalize_path(&cwd)) {
            continue;
        }
        if transcripts.is_none() {
            transcripts = Some(transcript_map(&home.join("projects"))?);
        }
        let transcripts = transcripts
            .as_ref()
            .expect("Cursor transcript map initialized");
        let transcript = transcripts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| path.clone());
        let title = meta
            .title
            .filter(|title| !title.trim().is_empty())
            .or_else(|| first_user(&transcript).ok().flatten())
            .unwrap_or_else(|| "Untitled Cursor session".to_owned());
        sessions.push(Session {
            id,
            agent: Agent::Cursor,
            project: project_name(&cwd),
            repository: None,
            branch: None,
            cwd,
            title: clean_text(&title, 72),
            status: SessionStatus::Stale,
            last_activity: DateTime::<Utc>::from_timestamp_millis(meta.updated_at_ms)
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            transcript,
            preview: String::new(),
            archived: false,
            parse_error: None,
            preview_loaded: false,
        });
    }
    Ok(sessions)
}

pub(crate) fn load_preview(path: &Path) -> Result<String> {
    if path.file_name().is_some_and(|name| name == "meta.json") {
        return Ok(String::new());
    }
    Ok(tail_values(path)?
        .iter()
        .rev()
        .filter(|value| value.get("role").and_then(Value::as_str) == Some("assistant"))
        .find_map(|value| value.get("message").and_then(message_text))
        .map(|text| clean_text(&text, 320))
        .unwrap_or_default())
}

fn first_user(path: &Path) -> Result<Option<String>> {
    if path.file_name().is_some_and(|name| name == "meta.json") {
        return Ok(None);
    }
    Ok(head_values(path)?
        .iter()
        .filter(|value| value.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|value| value.get("message").and_then(message_text))
        .find(|text| useful_user_text(text))
        .map(|text| clean_text(&text, 72)))
}

fn transcript_map(root: &Path) -> Result<HashMap<String, PathBuf>> {
    let mut map = HashMap::new();
    for path in jsonl_files(root)? {
        if path
            .components()
            .any(|component| component.as_os_str() == "agent-transcripts")
            && let Some(id) = path.file_stem()
        {
            map.insert(id.to_string_lossy().into_owned(), path);
        }
    }
    Ok(map)
}

fn named_files(root: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    visit_named(root, name, &mut files)?;
    Ok(files)
}

fn visit_named(directory: &Path, name: &str, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            visit_named(&path, name, files)?;
        } else if path.file_name().is_some_and(|file_name| file_name == name) {
            files.push(path);
        }
    }
    Ok(())
}

fn wsl_to_windows(path: &str) -> PathBuf {
    let bytes = path.as_bytes();
    if bytes.len() >= 7 && path.starts_with("/mnt/") && bytes[5].is_ascii_alphabetic() {
        let drive = (bytes[5] as char).to_ascii_uppercase();
        let rest = path[6..].replace('/', "\\");
        return PathBuf::from(format!("{drive}:{rest}"));
    }
    PathBuf::from(path)
}

fn project_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorMeta {
    #[serde(default)]
    has_conversation: bool,
    #[serde(default)]
    title: Option<String>,
    updated_at_ms: i64,
    cwd: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_wsl_workspace_path() {
        assert_eq!(
            wsl_to_windows("/mnt/x/workspace/project"),
            PathBuf::from(r"X:\workspace\project")
        );
    }
}
