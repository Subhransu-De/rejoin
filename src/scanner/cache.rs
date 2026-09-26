use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Agent, Session, SessionStatus};

const CACHE_VERSION: u8 = 3;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    length: u64,
    modified_ns: u64,
    session: Session,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TitleCache {
    fingerprint: (u64, u64),
    titles: HashMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u8,
    #[serde(default)]
    titles: HashMap<PathBuf, TitleCache>,
    entries: HashMap<PathBuf, CacheEntry>,
    sources: HashMap<PathBuf, Option<(u64, u64)>>,
}

#[derive(Debug)]
pub struct SessionCache {
    path: Option<PathBuf>,
    entries: HashMap<PathBuf, CacheEntry>,
    sources: HashMap<PathBuf, Option<(u64, u64)>>,
    dirty: AtomicBool,
    titles: Mutex<HashMap<PathBuf, TitleCache>>,
    fingerprints: Mutex<HashMap<PathBuf, (u64, u64)>>,
    warnings: Mutex<Vec<String>>,
}

impl SessionCache {
    #[cfg(test)]
    pub fn test_cache() -> Self {
        Self {
            path: None,
            entries: HashMap::new(),
            sources: HashMap::new(),
            dirty: AtomicBool::new(true),
            titles: Mutex::new(HashMap::new()),
            fingerprints: Mutex::new(HashMap::new()),
            warnings: Mutex::new(Vec::new()),
        }
    }

    pub fn load(options: &super::ScanOptions) -> Self {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for path in [
            &options.claude_home,
            &options.codex_home,
            &options.cursor_home,
            &options.pi_sessions,
            &options.opencode_database,
        ] {
            std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.clone())
                .hash(&mut hash);
        }
        let sources = [
            options.claude_home.join("history.jsonl"),
            options.codex_home.join("history.jsonl"),
            options.codex_home.join("session_index.jsonl"),
        ]
        .into_iter()
        .map(|path| {
            let stamp = fingerprint(&path);
            (path, stamp)
        })
        .collect::<HashMap<_, _>>();
        let namespace = hash.finish();
        let path = dirs::cache_dir().map(|directory| {
            directory
                .join("rejoin")
                .join(format!("sessions-v{CACHE_VERSION}-{namespace:016x}.json"))
        });
        let Some(path) = path else {
            return Self {
                path: None,
                entries: HashMap::new(),
                sources,
                dirty: AtomicBool::new(false),
                titles: Mutex::new(HashMap::new()),
                fingerprints: Mutex::new(HashMap::new()),
                warnings: Mutex::new(Vec::new()),
            };
        };
        let cache = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
            .filter(|cache| cache.version == CACHE_VERSION);
        match cache {
            Some(mut cache) => {
                let changed = cache.sources != sources;
                if changed {
                    cache.entries.retain(|_, entry| {
                        !matches!(entry.session.agent, Agent::Claude | Agent::Codex)
                    });
                }
                Self {
                    path: Some(path),
                    entries: cache.entries,
                    sources,
                    dirty: AtomicBool::new(changed),
                    titles: Mutex::new(cache.titles),
                    fingerprints: Mutex::new(HashMap::new()),
                    warnings: Mutex::new(Vec::new()),
                }
            }
            None => Self {
                path: Some(path),
                entries: HashMap::new(),
                sources,
                dirty: AtomicBool::new(true),
                titles: Mutex::new(HashMap::new()),
                fingerprints: Mutex::new(HashMap::new()),
                warnings: Mutex::new(Vec::new()),
            },
        }
    }

    pub fn titles(
        &self,
        path: &Path,
        read: impl FnOnce(&Path) -> HashMap<String, String>,
    ) -> HashMap<String, String> {
        let before = fingerprint(path);
        if let Some(cached) = self
            .titles
            .lock()
            .unwrap()
            .get(path)
            .filter(|entry| Some(entry.fingerprint) == before)
        {
            return cached.titles.clone();
        }
        let titles = read(path);
        if let Some(before) = before
            && fingerprint(path) == Some(before)
        {
            self.titles.lock().unwrap().insert(
                path.to_path_buf(),
                TitleCache {
                    fingerprint: before,
                    titles: titles.clone(),
                },
            );
            self.dirty.store(true, Ordering::Relaxed);
        }
        titles
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.warnings.lock().unwrap().push(message.into());
    }

    pub fn warnings(&self) -> Vec<String> {
        self.warnings.lock().unwrap().clone()
    }

    pub fn get(&self, path: &Path) -> Option<Session> {
        let fingerprint = fingerprint(path)?;
        self.fingerprints
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), fingerprint);
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
                if self.fingerprints.lock().unwrap().get(&session.transcript)
                    != Some(&(length, modified_ns))
                {
                    continue;
                }
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
            titles: self.titles.lock().unwrap().clone(),
            entries,
            sources: self.sources.clone(),
        };
        let parent = path
            .parent()
            .context("cache path has no parent directory")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
        let bytes = serde_json::to_vec(&cache)?;
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temporary = path.with_extension(format!("{}-{unique}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.with_context(|| format!("could not write {}", path.display()))
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
            sources: HashMap::new(),
            entries: HashMap::from([(
                transcript.clone(),
                CacheEntry {
                    length,
                    modified_ns,
                    session,
                },
            )]),
            dirty: AtomicBool::new(false),
            titles: Mutex::new(HashMap::new()),
            fingerprints: Mutex::new(HashMap::new()),
            warnings: Mutex::new(Vec::new()),
        };

        assert!(cache.get(&transcript).is_some());
        fs::write(&transcript, "{\"changed\":true}\n").unwrap();
        assert!(cache.get(&transcript).is_none());
        assert!(cache.dirty.load(Ordering::Relaxed));
    }

    #[test]
    fn does_not_cache_metadata_when_transcript_changes_during_scan() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.jsonl");
        fs::write(
            &path,
            r#"{"type":"session","id":"synthetic","cwd":"/synthetic"}"#,
        )
        .unwrap();
        let mut cache = SessionCache::test_cache();
        cache.path = Some(directory.path().join("cache.json"));
        let sessions = super::super::pi::scan(directory.path(), &cache).unwrap();
        assert_eq!(sessions.len(), 1);
        let mut session = sessions[0].clone();
        session.parse_error = None;
        session.title = "Synthetic old title".into();
        fs::write(&path, "new synthetic contents").unwrap();
        cache.save_if_dirty(&[session]).unwrap();
        let saved: CacheFile =
            serde_json::from_slice(&fs::read(cache.path.as_ref().unwrap()).unwrap()).unwrap();
        assert!(saved.entries.is_empty());
        cache.save_if_dirty(&[]).unwrap();
        assert!(
            serde_json::from_slice::<CacheFile>(&fs::read(cache.path.as_ref().unwrap()).unwrap())
                .is_ok()
        );
    }
}
