use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Agent, Session, SessionStatus};

const CACHE_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    length: u64,
    modified_ns: u64,
    session: Session,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u8,
    entries: HashMap<PathBuf, CacheEntry>,
}

#[derive(Debug)]
pub struct SessionCache {
    path: Option<PathBuf>,
    entries: HashMap<PathBuf, CacheEntry>,
    dirty: AtomicBool,
}

impl SessionCache {
    pub fn load() -> Self {
        let path = dirs::cache_dir().map(|directory| {
            directory
                .join("rejoin")
                .join(format!("sessions-v{CACHE_VERSION}.json"))
        });
        let Some(path) = path else {
            return Self {
                path: None,
                entries: HashMap::new(),
                dirty: AtomicBool::new(false),
            };
        };
        let cache = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
            .filter(|cache| cache.version == CACHE_VERSION);
        match cache {
            Some(cache) => Self {
                path: Some(path),
                entries: cache.entries,
                dirty: AtomicBool::new(false),
            },
            None => Self {
                path: Some(path),
                entries: HashMap::new(),
                dirty: AtomicBool::new(true),
            },
        }
    }

    pub fn get(&self, path: &Path) -> Option<Session> {
        let fingerprint = fingerprint(path)?;
        let entry = self.entries.get(path);
        let Some(entry) = entry
            .filter(|entry| entry.length == fingerprint.0 && entry.modified_ns == fingerprint.1)
        else {
            self.dirty.store(true, Ordering::Relaxed);
            return None;
        };
        let mut session = entry.session.clone();
        session.status = SessionStatus::Stale;
        session.preview.clear();
        session.preview_loaded = false;
        session.parse_error = None;
        Some(session)
    }

    pub fn save_if_dirty(&self, sessions: &[Session]) -> Result<()> {
        if !self.dirty.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut entries = HashMap::with_capacity(sessions.len());
        for session in sessions.iter().filter(|session| {
            session.parse_error.is_none()
                && matches!(session.agent, Agent::Claude | Agent::Codex | Agent::Pi)
        }) {
            if let Some((length, modified_ns)) = fingerprint(&session.transcript) {
                let mut cached = session.clone();
                cached.status = SessionStatus::Stale;
                cached.preview.clear();
                cached.preview_loaded = false;
                entries.insert(
                    session.transcript.clone(),
                    CacheEntry {
                        length,
                        modified_ns,
                        session: cached,
                    },
                );
            }
        }
        let cache = CacheFile {
            version: CACHE_VERSION,
            entries,
        };
        let parent = path
            .parent()
            .context("cache path has no parent directory")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
        let bytes = serde_json::to_vec(&cache)?;
        std::fs::write(path, bytes).with_context(|| format!("could not write {}", path.display()))
    }
}

fn fingerprint(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let nanos = modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64;
    Some((metadata.len(), nanos))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;

    use super::*;
    use crate::model::{Agent, SessionStatus};

    #[test]
    fn invalidates_entry_when_transcript_changes() {
        let directory = tempfile::tempdir().unwrap();
        let transcript = directory.path().join("session.jsonl");
        fs::write(&transcript, "{}\n").unwrap();
        let (length, modified_ns) = fingerprint(&transcript).unwrap();
        let session = Session {
            id: "session-1".to_owned(),
            agent: Agent::Codex,
            project: "rejoin".to_owned(),
            repository: Some("rejoin".to_owned()),
            branch: Some("main".to_owned()),
            cwd: directory.path().to_path_buf(),
            title: "Cache test".to_owned(),
            status: SessionStatus::Idle,
            last_activity: Utc::now(),
            transcript: transcript.clone(),
            preview: String::new(),
            archived: false,
            parse_error: None,
            preview_loaded: false,
        };
        let cache = SessionCache {
            path: None,
            entries: HashMap::from([(
                transcript.clone(),
                CacheEntry {
                    length,
                    modified_ns,
                    session,
                },
            )]),
            dirty: AtomicBool::new(false),
        };

        assert!(cache.get(&transcript).is_some());
        fs::write(&transcript, "{\"changed\":true}\n").unwrap();
        assert!(cache.get(&transcript).is_none());
        assert!(cache.dirty.load(Ordering::Relaxed));
    }
}
