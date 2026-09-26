use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

// Metadata is at the beginning of both formats. A bounded preview keeps startup
// proportional to the number of sessions instead of the size of years of logs.
const HEAD_BYTES: u64 = 256 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;

pub fn head_values(path: &Path) -> Result<Vec<Value>> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let length = file.metadata()?.len();
    let mut bytes = Vec::new();
    file.take(HEAD_BYTES).read_to_end(&mut bytes)?;
    if length > HEAD_BYTES {
        bytes.truncate(
            bytes
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |i| i + 1),
        );
    }
    Ok(bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect())
}

pub fn tail_values(path: &Path) -> Result<Vec<Value>> {
    let mut file =
        File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let length = file.metadata()?.len();
    let offset = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(offset.saturating_sub(1)))?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES + 1).read_to_end(&mut bytes)?;
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
    prompt_text(text).is_some()
}

pub(crate) fn prompt_text(text: &str) -> Option<String> {
    let mut text = text.trim().to_owned();
    if text.starts_with("# AGENTS.md instructions") || text == "Warmup" {
        return None;
    }
    if let Some(start) = text.find("<user_query>") {
        text = text[start + 12..]
            .split("</user_query>")
            .next()?
            .trim()
            .to_owned();
    }
    for tag in [
        "system-reminder",
        "environment_context",
        "user_instructions",
        "permissions",
        "timestamp",
        "local-command-stdout",
        "skill",
    ] {
        while let Some(start) = text.find(&format!("<{tag}")) {
            let Some(end) = text[start..].find(&format!("</{tag}>")) else {
                text.truncate(start);
                break;
            };
            text.replace_range(start..start + end + tag.len() + 3, "");
        }
    }
    if text.trim_start().starts_with("Caveat:") {
        return None;
    }
    if let Some(start) = text.find("<command-name>") {
        text = text[start + 14..]
            .split("</command-name>")
            .next()?
            .to_owned();
    }
    nonempty(&text)
}

fn nonempty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_reads_drop_oversized_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.jsonl");
        std::fs::write(
            &path,
            format!(
                "{{\"id\":\"synthetic\"}}\n{}\n",
                serde_json::json!({"text":"x".repeat(1024*1024)})
            ),
        )
        .unwrap();
        let records = head_values(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["id"], "synthetic");
    }
    #[test]
    fn prompt_wrappers_do_not_become_titles() {
        assert_eq!(
            prompt_text("<timestamp>synthetic</timestamp><user_query>Actual task</user_query>")
                .as_deref(),
            Some("Actual task")
        );
        assert!(prompt_text("<environment_context>synthetic</environment_context>").is_none());
    }
}
