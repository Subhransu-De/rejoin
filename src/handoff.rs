use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;

use crate::model::{Agent, Handoff, Session};

const MAX_PROGRESS_CHARS: usize = 4_000;
const MAX_TASK_CHARS: usize = 3_000;
const MAX_ITEMS: usize = 30;

#[derive(Default)]
struct Evidence {
    task: Option<String>,
    progress: Vec<String>,
    decisions: Vec<String>,
    files: Vec<String>,
    commands: Vec<String>,
    remaining: Vec<String>,
}

pub fn generate(session: &Session) -> Result<Handoff> {
    let mut evidence = Evidence::default();

    if session.agent == Agent::OpenCode {
        for (role, text) in crate::scanner::opencode_text_history(&session.transcript, &session.id)?
        {
            consume_role_text(&role, &text, &mut evidence);
        }
        for (name, input) in
            crate::scanner::opencode_tool_history(&session.transcript, &session.id)?
        {
            consume_encoded_tool(&name, &Value::String(input), &mut evidence);
        }
    } else {
        let file = File::open(&session.transcript)
            .with_context(|| format!("could not open {}", session.transcript.display()))?;
        for line in BufReader::new(file).lines() {
            let line = line?;
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match session.agent {
                Agent::Claude => consume_claude(&value, &mut evidence),
                Agent::Codex => consume_codex(&value, &mut evidence),
                Agent::Cursor => consume_cursor(&value, &mut evidence),
                Agent::Pi => consume_pi(&value, &mut evidence),
                Agent::OpenCode => unreachable!(),
            }
        }
    }

    deduplicate(&mut evidence.files);
    deduplicate(&mut evidence.commands);
    deduplicate(&mut evidence.decisions);
    deduplicate(&mut evidence.remaining);
    trim_to_last(&mut evidence.files, MAX_ITEMS);
    trim_to_last(&mut evidence.commands, MAX_ITEMS);
    trim_to_last(&mut evidence.decisions, 8);
    trim_to_last(&mut evidence.remaining, 8);
    trim_to_last(&mut evidence.progress, 3);

    let markdown = render(session, &evidence);
    Ok(Handoff {
        markdown,
        suggested_name: format!("HANDOFF-{}.md", slugify(&session.title)),
    })
}

fn consume_cursor(value: &Value, evidence: &mut Evidence) {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(message) = value.get("message") else {
        return;
    };
    consume_message(role, message, evidence);
}

fn consume_pi(value: &Value, evidence: &mut Evidence) {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return;
    }
    let Some(message) = value.get("message") else {
        return;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    consume_message(role, message, evidence);
}

fn consume_message(role: &str, message: &Value, evidence: &mut Evidence) {
    if let Some(text) = text_content(message) {
        consume_role_text(role, &text, evidence);
    }
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            let block_type = block.get("type").and_then(Value::as_str);
            if !matches!(block_type, Some("tool_use" | "toolCall")) {
                continue;
            }
            let name = block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let input = block
                .get("input")
                .or_else(|| block.get("arguments"))
                .unwrap_or(&Value::Null);
            consume_encoded_tool(name, input, evidence);
        }
    }
}

fn consume_role_text(role: &str, text: &str, evidence: &mut Evidence) {
    if role == "user" && evidence.task.is_none() && useful_task(text) {
        evidence.task = Some(limit(text, MAX_TASK_CHARS));
    } else if role == "assistant" {
        consume_assistant_text(text, evidence);
    }
}

pub fn save(handoff: &Handoff, cwd: &Path) -> Result<PathBuf> {
    let directory = if cwd.is_dir() { cwd } else { Path::new(".") };
    let desired = directory.join(&handoff.suggested_name);
    let path = unique_path(desired);
    std::fs::write(&path, &handoff.markdown)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(path)
}

fn consume_claude(value: &Value, evidence: &mut Evidence) {
    let record_type = value.get("type").and_then(Value::as_str);
    if !matches!(record_type, Some("user" | "assistant")) {
        return;
    }
    let Some(message) = value.get("message") else {
        return;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if role == "user"
        && evidence.task.is_none()
        && let Some(text) = text_content(message)
        && useful_task(&text)
    {
        evidence.task = Some(limit(&text, MAX_TASK_CHARS));
    } else if role == "assistant"
        && let Some(text) = text_content(message)
    {
        consume_assistant_text(&text, evidence);
    }

    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let name = block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let input = block.get("input").unwrap_or(&Value::Null);
            consume_tool(name, input, evidence);
        }
    }
}

fn consume_codex(value: &Value, evidence: &mut Evidence) {
    let outer_type = value.get("type").and_then(Value::as_str);
    if outer_type == Some("message") {
        let role = value.get("role").and_then(Value::as_str);
        if role == Some("user")
            && evidence.task.is_none()
            && let Some(text) = text_content(value)
            && useful_task(&text)
        {
            evidence.task = Some(limit(&text, MAX_TASK_CHARS));
        } else if role == Some("assistant")
            && let Some(text) = text_content(value)
        {
            consume_assistant_text(&text, evidence);
        }
        return;
    }
    if matches!(
        outer_type,
        Some("function_call" | "custom_tool_call" | "local_shell_call")
    ) {
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(outer_type.unwrap_or_default());
        let input = value
            .get("arguments")
            .or_else(|| value.get("input"))
            .unwrap_or(&Value::Null);
        consume_encoded_tool(name, input, evidence);
        return;
    }
    let Some(payload) = value.get("payload") else {
        return;
    };
    let payload_type = payload.get("type").and_then(Value::as_str);

    if outer_type == Some("event_msg")
        && payload_type == Some("user_message")
        && evidence.task.is_none()
        && let Some(text) = payload.get("message").and_then(Value::as_str)
        && useful_task(text)
    {
        evidence.task = Some(limit(text, MAX_TASK_CHARS));
    } else if outer_type == Some("event_msg") && payload_type == Some("agent_message") {
        if let Some(text) = payload.get("message").and_then(Value::as_str) {
            consume_assistant_text(text, evidence);
        }
    } else if outer_type == Some("response_item")
        && matches!(
            payload_type,
            Some("function_call" | "custom_tool_call" | "local_shell_call")
        )
    {
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(payload_type.unwrap_or_default());
        let input = payload
            .get("arguments")
            .or_else(|| payload.get("input"))
            .unwrap_or(&Value::Null);
        consume_encoded_tool(name, input, evidence);
    }
}

fn consume_encoded_tool(name: &str, input: &Value, evidence: &mut Evidence) {
    let decoded;
    let input = if let Some(json) = input.as_str() {
        decoded = serde_json::from_str(json).unwrap_or_else(|_| Value::String(json.to_owned()));
        &decoded
    } else {
        input
    };
    consume_tool(name, input, evidence);
}

fn consume_assistant_text(text: &str, evidence: &mut Evidence) {
    let clean = text.trim();
    if clean.is_empty() {
        return;
    }
    evidence.progress.push(limit(clean, MAX_PROGRESS_CHARS));
    for line in clean.lines().map(str::trim) {
        let lower = line.to_lowercase();
        if lower.contains("decision")
            || lower.starts_with("chose ")
            || lower.starts_with("using ")
            || lower.contains("we'll use")
        {
            evidence.decisions.push(strip_bullet(line));
        }
        if lower.contains("remaining")
            || lower.contains("next step")
            || lower.starts_with("todo")
            || lower.starts_with("- [ ]")
        {
            evidence.remaining.push(strip_bullet(line));
        }
    }
}

fn consume_tool(name: &str, input: &Value, evidence: &mut Evidence) {
    let lower = name.to_lowercase();
    if (lower.contains("shell") || lower == "bash" || lower == "powershell")
        && let Some(command) = input
            .get("command")
            .or_else(|| input.get("cmd"))
            .and_then(Value::as_str)
            .or_else(|| input.as_str())
    {
        evidence.commands.push(limit(command, 800));
    }

    for key in ["file_path", "path", "workdir"] {
        if let Some(path) = input.get(key).and_then(Value::as_str)
            && (key != "workdir" || lower.contains("edit") || lower.contains("write"))
        {
            evidence.files.push(path.to_owned());
        }
    }

    if lower.contains("patch")
        && let Some(patch) = input
            .as_str()
            .or_else(|| input.get("patch").and_then(Value::as_str))
            .or_else(|| input.get("patchText").and_then(Value::as_str))
    {
        for line in patch.lines() {
            for prefix in [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ] {
                if let Some(path) = line.strip_prefix(prefix) {
                    evidence.files.push(path.trim().to_owned());
                }
            }
        }
    }
}

fn render(session: &Session, evidence: &Evidence) -> String {
    let repository = session.repository.as_deref().unwrap_or("not detected");
    let branch = session.branch.as_deref().unwrap_or("not recorded");
    let task = evidence
        .task
        .as_deref()
        .unwrap_or("No user task could be extracted from the transcript.");
    let progress = if evidence.progress.is_empty() {
        "No progress summary could be extracted. Inspect the transcript and repository state."
            .to_owned()
    } else {
        evidence.progress.join("\n\n")
    };

    format!(
        "# Handoff: {title}\n\n\
         **Source agent:** {agent}  \n\
         **Session:** `{id}`  \n\
         **Project:** {project}  \n\
         **Repository:** {repository}  \n\
         **Branch:** `{branch}`  \n\
         **Working directory:** `{cwd}`  \n\
         **Generated:** {generated}\n\n\
         ## Task\n\n{task}\n\n\
         ## Current progress\n\n{progress}\n\n\
         ## Decisions\n\n{decisions}\n\n\
         ## Relevant files\n\n{files}\n\n\
         ## Commands run\n\n{commands}\n\n\
         ## Remaining work\n\n{remaining}\n\n\
         ## Continuation note\n\n\
         Verify the current repository state before making changes. This package is extracted \
         from the source session and may omit context that was not recorded in its transcript.\n",
        title = session.title,
        agent = session.agent,
        id = session.id,
        project = session.project,
        cwd = session.cwd.display(),
        generated = Utc::now().to_rfc3339(),
        decisions = bullets(&evidence.decisions, "No explicit decisions detected."),
        files = code_bullets(&evidence.files, "No relevant files detected."),
        commands = code_bullets(&evidence.commands, "No shell commands detected."),
        remaining = bullets(
            &evidence.remaining,
            "Review the latest progress and continue from the current repository state."
        ),
    )
}

fn text_content(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let text = content
        .as_array()?
        .iter()
        .filter(|block| {
            matches!(
                block.get("type").and_then(Value::as_str),
                None | Some("text" | "input_text" | "output_text")
            )
        })
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn useful_task(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !text.starts_with("<system-reminder>")
        && !text.starts_with("<environment_context>")
        && !text.starts_with("# AGENTS.md instructions")
        && text != "Warmup"
}

fn bullets(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_owned()
    } else {
        items
            .iter()
            .map(|item| format!("- {}", item.replace('\n', "\n  ")))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn code_bullets(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_owned()
    } else {
        items
            .iter()
            .map(|item| format!("- `{}`", item.replace('`', "\\`").replace('\n', " ")))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn deduplicate(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.trim().to_owned()));
}

fn trim_to_last(items: &mut Vec<String>, count: usize) {
    if items.len() > count {
        items.drain(..items.len() - count);
    }
}

fn strip_bullet(line: &str) -> String {
    line.trim_start_matches(['-', '*', ' ', '\t'])
        .trim()
        .to_owned()
}

fn limit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.trim().to_owned();
    }
    let mut output = text.chars().take(max.saturating_sub(1)).collect::<String>();
    output.push('…');
    output
}

fn slugify(title: &str) -> String {
    let slug = title
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "session".to_owned()
    } else {
        slug
    }
}

fn unique_path(desired: PathBuf) -> PathBuf {
    if !desired.exists() {
        return desired;
    }
    let parent = desired.parent().unwrap_or(Path::new(""));
    let stem = desired
        .file_stem()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    let extension = desired
        .extension()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    for suffix in 2..10_000 {
        let file = if extension.is_empty() {
            format!("{stem}-{suffix}")
        } else {
            format!("{stem}-{suffix}.{extension}")
        };
        let candidate = parent.join(file);
        if !candidate.exists() {
            return candidate;
        }
    }
    desired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_safe_and_short() {
        assert_eq!(slugify("Fix: Resume Race!"), "fix-resume-race");
        assert_eq!(slugify("***"), "session");
    }

    #[test]
    fn extracts_patch_paths() {
        let mut evidence = Evidence::default();
        consume_tool(
            "apply_patch",
            &Value::String("*** Update File: src/main.rs\n*** Add File: src/ui.rs".to_owned()),
            &mut evidence,
        );
        assert_eq!(evidence.files, ["src/main.rs", "src/ui.rs"]);
    }
}
