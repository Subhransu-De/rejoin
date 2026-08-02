use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde_json::Value;

// Metadata is at the beginning of both formats. A bounded preview keeps startup
// proportional to the number of sessions instead of the size of years of logs.
const HEAD_BYTES: u64 = 256 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;

pub fn head_values(path: &Path) -> Result<Vec<Value>> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut values = Vec::new();
    let mut consumed = 0_u64;
    for line in BufReader::new(file).lines() {
        let line = line?;
        consumed += line.len() as u64 + 1;
        if let Ok(value) = serde_json::from_str(&line) {
            values.push(value);
        }
        if consumed >= HEAD_BYTES {
            break;
        }
    }
    Ok(values)
}

pub fn first_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let line = BufReader::new(file)
        .lines()
        .next()
        .transpose()?
        .context("session transcript is empty")?;
    serde_json::from_str(&line).with_context(|| format!("invalid JSON in {}", path.display()))
}

pub fn tail_values(path: &Path) -> Result<Vec<Value>> {
    let mut file =
        File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let length = file.metadata()?.len();
    let offset = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    // A bounded tail can begin in the middle of a UTF-8 code point. Lossy
    // decoding only affects that discarded partial line and keeps the session
    // discoverable.
    let content = String::from_utf8_lossy(&bytes);

    Ok(content
        .lines()
        .skip(usize::from(offset > 0))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

pub fn modified_time(path: &Path) -> DateTime<Utc> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| DateTime::<Utc>::from(SystemTime::UNIX_EPOCH))
}

pub fn message_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return nonempty(text);
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return nonempty(text);
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = content.as_str() {
            return nonempty(text);
        }
        if let Some(items) = content.as_array() {
            let text = items
                .iter()
                .filter_map(|item| {
                    if matches!(
                        item.get("type").and_then(Value::as_str),
                        None | Some("text" | "input_text" | "output_text")
                    ) {
                        item.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            return nonempty(&text);
        }
    }
    None
}

pub fn clean_text(text: &str, max_chars: usize) -> String {
    let flattened = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned();
    if flattened.chars().count() <= max_chars {
        flattened
    } else {
        let mut shortened = flattened
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        shortened.push('…');
        shortened
    }
}

pub fn useful_user_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with("<system-reminder>")
        && !trimmed.starts_with("<environment_context>")
        && !trimmed.starts_with("# AGENTS.md instructions")
        && trimmed != "Warmup"
}

fn nonempty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
