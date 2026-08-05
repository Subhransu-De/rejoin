use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Agent, Session, SessionStatus};

use super::common::{clean_text, head_values, message_text, tail_values, useful_user_text};
use super::parallel_map;

pub fn scan(home: &Path, scope: Option<&Path>) -> Result<Vec<Session>> {
    if !home.join("chats").exists() {
        return Ok(Vec::new());
    }
    let normalized_scope = scope.map(super::normalize_path);
    let mut transcript_roots = None;
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
        if normalized_scope
            .as_ref()
            .is_some_and(|scope| scope != &super::normalize_path(&cwd))
        {
            continue;
        }
        let roots = transcript_roots
            .get_or_insert_with(|| find_transcript_roots(&home.join("projects")))
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error:#}"))?;
        let transcript = resolve_transcript_in_roots(&path, &id, roots);
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

pub(crate) fn resolve_transcript(path: &Path, session_id: &str) -> Result<PathBuf> {
    if !path.file_name().is_some_and(|name| name == "meta.json") {
        return Ok(path.to_path_buf());
    }
    let Some(cursor_home) = path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "chats"))
        .and_then(Path::parent)
    else {
        return Ok(path.to_path_buf());
    };
    let roots = find_transcript_roots(&cursor_home.join("projects"))?;
    Ok(resolve_transcript_in_roots(path, session_id, &roots))
}

fn find_transcript_roots(projects: &Path) -> Result<Vec<PathBuf>> {
    if !projects.exists() {
        return Ok(Vec::new());
    }
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(projects)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            roots.push(entry.path().join("agent-transcripts"));
        }
    }
    Ok(roots)
}

fn resolve_transcript_in_roots(path: &Path, session_id: &str, roots: &[PathBuf]) -> PathBuf {
    for root in roots {
        let candidate = root.join(session_id).join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            return candidate;
        }
    }
    path.to_path_buf()
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
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit_named(&path, name, files)?;
        } else if file_type.is_file() && path.file_name().is_some_and(|file_name| file_name == name)
        {
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
    use std::fs;

    use super::*;

    #[test]
    fn converts_wsl_workspace_path() {
        assert_eq!(
            wsl_to_windows("/mnt/x/workspace/project"),
            PathBuf::from(r"X:\workspace\project")
        );
    }

    #[test]
    fn resolves_transcript_lazily_from_cursor_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let metadata = directory
            .path()
            .join("chats")
            .join("workspace")
            .join("session-id")
            .join("meta.json");
        let transcript = directory
            .path()
            .join("projects")
            .join("project")
            .join("agent-transcripts")
            .join("session-id")
            .join("session-id.jsonl");
        fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(&metadata, "{}").unwrap();
        fs::write(&transcript, "{}\n").unwrap();

        assert_eq!(
            resolve_transcript(&metadata, "session-id").unwrap(),
            transcript
        );
    }
}
