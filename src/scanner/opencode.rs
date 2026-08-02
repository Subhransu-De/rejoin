use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};

use crate::model::{Agent, Session, SessionStatus};

pub fn scan(database: &Path) -> Result<Vec<Session>> {
    if !database.exists() {
        return Ok(Vec::new());
    }
    let connection = open(database)?;
    let mut statement = connection.prepare(
        "SELECT s.id, s.directory, s.title, s.time_updated, s.time_archived \
         FROM session s ORDER BY s.time_updated DESC",
    )?;
    let rows = statement.query_map([], |row| {
        let cwd = PathBuf::from(row.get::<_, String>(1)?);
        Ok(Session {
            id: row.get(0)?,
            agent: Agent::OpenCode,
            project: cwd
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_owned()),
            repository: None,
            branch: None,
            cwd,
            title: row.get(2)?,
            status: SessionStatus::Stale,
            last_activity: DateTime::<Utc>::from_timestamp_millis(row.get(3)?)
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            transcript: database.to_path_buf(),
            preview: String::new(),
            archived: row.get::<_, Option<i64>>(4)?.is_some(),
            parse_error: None,
            preview_loaded: false,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("could not read OpenCode sessions")
}

pub(crate) fn load_preview(database: &Path, session_id: &str) -> Result<String> {
    let connection = open(database)?;
    let preview = connection.query_row(
        "SELECT json_extract(p.data, '$.text') \
         FROM part p JOIN message m ON m.id = p.message_id \
         WHERE p.session_id = ?1 \
           AND json_extract(p.data, '$.type') = 'text' \
           AND json_extract(m.data, '$.role') = 'assistant' \
         ORDER BY p.time_updated DESC LIMIT 1",
        [session_id],
        |row| row.get::<_, String>(0),
    );
    match preview {
        Ok(preview) => Ok(super::common::clean_text(&preview, 320)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn text_history(database: &Path, session_id: &str) -> Result<Vec<(String, String)>> {
    let connection = open(database)?;
    let mut statement = connection.prepare(
        "SELECT json_extract(m.data, '$.role'), json_extract(p.data, '$.text') \
         FROM part p JOIN message m ON m.id = p.message_id \
         WHERE p.session_id = ?1 AND json_extract(p.data, '$.type') = 'text' \
         ORDER BY p.time_created",
    )?;
    let rows = statement.query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn tool_history(database: &Path, session_id: &str) -> Result<Vec<(String, String)>> {
    let connection = open(database)?;
    let mut statement = connection.prepare(
        "SELECT json_extract(data, '$.tool'), json_extract(data, '$.state.input') \
         FROM part WHERE session_id = ?1 AND json_extract(data, '$.type') = 'tool' \
         ORDER BY time_created",
    )?;
    let rows = statement.query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn open(database: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("could not open {}", database.display()))
}
