use std::fmt;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
    Pi,
    OpenCode,
}

impl Agent {
    pub const ALL: [Self; 5] = [
        Self::Codex,
        Self::Claude,
        Self::Cursor,
        Self::Pi,
        Self::OpenCode,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::Pi => "Pi",
            Self::OpenCode => "OpenCode",
        }
    }

    pub fn binary(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor-agent",
            Self::Pi => "pi",
            Self::OpenCode => "opencode",
        }
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Idle,
    Stale,
    Error,
}

impl SessionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Active => "●",
            Self::Idle => "◐",
            Self::Stale => "○",
            Self::Error => "×",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent: Agent,
    pub project: String,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub cwd: PathBuf,
    pub title: String,
    pub status: SessionStatus,
    pub last_activity: DateTime<Utc>,
    pub transcript: PathBuf,
    pub preview: String,
    pub archived: bool,
    #[serde(skip)]
    pub parse_error: Option<String>,
    #[serde(skip)]
    pub preview_loaded: bool,
}

impl Session {
    pub fn search_text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {}",
            self.agent.label(),
            self.project,
            self.repository.as_deref().unwrap_or_default(),
            self.branch.as_deref().unwrap_or_default(),
            self.cwd.display(),
            self.title,
            self.preview
        )
        .to_lowercase()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Recency {
    #[default]
    Any,
    Day,
    Week,
    Month,
}

impl Recency {
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "any time",
            Self::Day => "24 hours",
            Self::Week => "7 days",
            Self::Month => "30 days",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Any => Self::Day,
            Self::Day => Self::Week,
            Self::Week => Self::Month,
            Self::Month => Self::Any,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Any => Self::Month,
            Self::Day => Self::Any,
            Self::Week => Self::Day,
            Self::Month => Self::Week,
        }
    }

    pub fn matches(self, time: DateTime<Utc>) -> bool {
        let age = Utc::now() - time;
        match self {
            Self::Any => true,
            Self::Day => age <= Duration::days(1),
            Self::Week => age <= Duration::days(7),
            Self::Month => age <= Duration::days(30),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Filters {
    pub project: String,
    pub repository: String,
    pub recency: Recency,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.project.is_empty() && self.repository.is_empty() && self.recency == Recency::Any
    }

    pub fn matches(&self, session: &Session) -> bool {
        let project = self.project.to_lowercase();
        let repository = self.repository.to_lowercase();
        (project.is_empty() || session.project.to_lowercase().contains(&project))
            && (repository.is_empty()
                || session
                    .repository
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&repository))
            && self.recency.matches(session.last_activity)
    }
}

#[derive(Clone, Debug)]
pub struct Handoff {
    pub markdown: String,
    pub suggested_name: String,
}

pub fn relative_time(time: DateTime<Utc>) -> String {
    let delta = Utc::now().signed_duration_since(time);
    if delta.num_seconds() < 60 {
        "now".to_owned()
    } else if delta.num_minutes() < 60 {
        format!("{}m", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h", delta.num_hours())
    } else if delta.num_days() < 30 {
        format!("{}d", delta.num_days())
    } else {
        time.format("%Y-%m-%d").to_string()
    }
}
