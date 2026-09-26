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
    instructions: Vec<String>,
    progress: Vec<String>,
    decisions: Vec<String>,
    files: Vec<String>,
    read_files: Vec<String>,
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
    deduplicate(&mut evidence.read_files);
    trim_to_last(&mut evidence.read_files, MAX_ITEMS);
    deduplicate(&mut evidence.commands);
    deduplicate(&mut evidence.decisions);
    deduplicate(&mut evidence.remaining);
    trim_to_last(&mut evidence.files, MAX_ITEMS);
    trim_to_last(&mut evidence.commands, MAX_ITEMS);
    trim_to_last(&mut evidence.decisions, 8);
    trim_to_last(&mut evidence.remaining, 8);
    trim_to_last(&mut evidence.progress, 3);

    let markdown = redact(&render(session, &evidence));
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
    if role == "user" {
        if let Some(text) = crate::scanner::prompt_text(text) {
            let text = limit(&redact_record(&text), MAX_TASK_CHARS);
            if evidence.task.is_none() {
                evidence.task = Some(text);
            } else if evidence.task.as_ref() != Some(&text)
                && evidence.instructions.last() != Some(&text)
            {
                evidence.instructions.push(text);
                trim_to_last(&mut evidence.instructions, 3);
            }
        }
    } else if role == "assistant" {
        consume_assistant_text(text, evidence);
    }
}

pub fn save(handoff: &Handoff, _cwd: &Path) -> Result<PathBuf> {
    let directory = dirs::cache_dir()
        .context("could not determine private handoff directory")?
        .join("rejoin")
        .join("handoffs");
    std::fs::create_dir_all(&directory)?;
    save_in(handoff, &directory)
}

fn save_in(handoff: &Handoff, directory: &Path) -> Result<PathBuf> {
    use std::io::Write;
    let safe_name = Path::new(&handoff.suggested_name)
        .file_name()
        .context("invalid handoff filename")?
        .to_string_lossy();
    let name = format!(
        "{}-{}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id(),
        safe_name
    );
    let path = directory.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(redact(&handoff.markdown).as_bytes())?;
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
    if let Some(text) = text_content(message) {
        consume_role_text(role, &text, evidence);
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
        if let Some(text) = text_content(value) {
            consume_role_text(
                value
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &text,
                evidence,
            );
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
            .or_else(|| value.get("action"))
            .unwrap_or(&Value::Null);
        consume_encoded_tool(name, input, evidence);
        return;
    }
    let Some(payload) = value.get("payload") else {
        return;
    };
    let payload_type = payload.get("type").and_then(Value::as_str);
    if outer_type == Some("response_item") && payload_type == Some("message") {
        if let Some(text) = text_content(payload) {
            consume_role_text(
                payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &text,
                evidence,
            );
        }
        return;
    }

    if outer_type == Some("event_msg")
        && payload_type == Some("user_message")
        && let Some(text) = payload.get("message").and_then(Value::as_str)
    {
        consume_role_text("user", text, evidence);
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
            .or_else(|| payload.get("action"))
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
    let sanitized = redact_record(text);
    let clean = sanitized.trim();
    if clean.is_empty() {
        return;
    }
    let item = limit(clean, MAX_PROGRESS_CHARS);
    if evidence.progress.last() != Some(&item) {
        evidence.progress.push(item);
    }
    trim_to_last(&mut evidence.progress, 3);
    for line in clean.lines().map(str::trim) {
        let lower = line.to_lowercase();
        if lower.contains("decision")
            || lower.starts_with("chose ")
            || lower.starts_with("using ")
            || lower.contains("we'll use")
        {
            evidence.decisions.push(limit(&strip_bullet(line), 800));
            trim_to_last(&mut evidence.decisions, 8);
        }
        if lower.contains("remaining")
            || lower.contains("next step")
            || lower.starts_with("todo")
            || lower.starts_with("- [ ]")
        {
            evidence.remaining.push(limit(&strip_bullet(line), 800));
            trim_to_last(&mut evidence.remaining, 8);
        }
    }
}

fn consume_tool(name: &str, input: &Value, evidence: &mut Evidence) {
    let lower = name.rsplit('.').next().unwrap_or(name).to_lowercase();
    if (lower.contains("shell")
        || lower == "bash"
        || lower == "powershell"
        || lower == "exec"
        || lower == "exec_command")
        && let Some(command) = input
            .get("command")
            .or_else(|| input.get("cmd"))
            .and_then(Value::as_str)
            .or_else(|| input.as_str())
    {
        evidence.commands.push(limit(&redact_record(command), 800));
    }

    if let Some(parts) = input.get("command").and_then(Value::as_array) {
        let parts = parts.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        let parts = if parts.len() >= 3 && matches!(parts[1], "-lc" | "-c") {
            &parts[2..]
        } else {
            &parts[..]
        };
        if lower.contains("shell") || lower == "bash" || lower == "exec" {
            evidence
                .commands
                .push(limit(&redact_record(&parts.join(" ")), 800));
        }
    }
    trim_to_last(&mut evidence.commands, MAX_ITEMS);
    for key in ["file_path", "path", "workdir"] {
        if let Some(path) = input.get(key).and_then(Value::as_str)
            && (key != "workdir" || lower.contains("edit") || lower.contains("write"))
        {
            if lower.contains("edit") || lower.contains("write") || lower.contains("patch") {
                evidence.files.push(path.to_owned());
            } else {
                evidence.read_files.push(path.to_owned());
            }
        }
    }

    if (lower.contains("patch") || lower == "exec")
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
    trim_to_last(&mut evidence.files, MAX_ITEMS);
    trim_to_last(&mut evidence.read_files, MAX_ITEMS);
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
         ## Recent instructions\n\n{instructions}\n\n\
         ## Current progress\n\n{progress}\n\n\
         ## Possible decisions (auto-detected)\n\n{decisions}\n\n\
         ## Changed files\n\n{files}\n\n\
         ## Read files\n\n{read_files}\n\n\
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
        instructions = bullets(&evidence.instructions, "No later instructions recorded."),
        read_files = code_bullets(&evidence.read_files, "No read paths detected."),
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
    let slug = slug.chars().take(80).collect::<String>();
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

fn redact_record(text: &str) -> String {
    if redact(text) != text.lines().collect::<Vec<_>>().join("\n") {
        "[REDACTED: potentially sensitive content]".to_owned()
    } else {
        text.to_owned()
    }
}

fn redact(text: &str) -> String {
    text.lines()
        .map(|line| {
            let lower = line.to_lowercase();
            let assignment = line.match_indices(['=', ':']).any(|(index, _)| {
                let prefix = line[..index].trim_end().trim_end_matches(['\'', '"']);
                let name = prefix
                    .rsplit(|character: char| {
                        !character.is_alphanumeric() && !matches!(character, '_' | '-')
                    })
                    .next()
                    .unwrap_or_default();
                let name = name.to_ascii_lowercase();
                ["key", "token", "secret", "password"]
                    .iter()
                    .any(|key| name.contains(key))
            });
            let sensitive = assignment
                || lower.contains("authorization:")
                || lower.contains("bearer ")
                || ["sk-", "ghp_", "github_pat_", "xoxb-", "xoxp-", "akia"]
                    .iter()
                    .any(|prefix| lower.contains(prefix))
                || line.split_whitespace().any(|word| {
                    word.split_once("://").is_some_and(|(_, rest)| {
                        rest.split('/')
                            .next()
                            .is_some_and(|authority| authority.contains('@'))
                    })
                });
            if sensitive {
                "[REDACTED: potentially sensitive content]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    #[test]
    fn modern_codex_handoff_keeps_messages_corrections_and_commands() {
        let mut evidence = Evidence::default();
        for (role, text) in [
            ("user", "Synthetic initial task"),
            ("assistant", "Synthetic progress"),
            ("user", "Synthetic correction"),
        ] {
            consume_codex(
                &serde_json::json!({"type":"response_item","payload":{"type":"message","role":role,"content":[{"type":"text","text":text}]}}),
                &mut evidence,
            );
        }
        consume_encoded_tool(
            "exec_command",
            &serde_json::json!({"cmd":"echo synthetic"}),
            &mut evidence,
        );
        consume_encoded_tool(
            "shell",
            &serde_json::json!({"command":["bash","-lc","echo fixture"]}),
            &mut evidence,
        );
        assert_eq!(evidence.task.as_deref(), Some("Synthetic initial task"));
        assert_eq!(evidence.progress, ["Synthetic progress"]);
        assert_eq!(evidence.instructions, ["Synthetic correction"]);
        assert!(
            evidence
                .commands
                .iter()
                .any(|command| command.contains("echo synthetic"))
        );
        assert!(
            evidence
                .commands
                .iter()
                .any(|command| command.contains("echo fixture"))
        );
    }

    #[test]
    fn redacts_synthetic_credentials_in_all_text_sections() {
        for text in [
            "export API_KEY=synthetic-secret",
            "curl -H \"Authorization: Bearer synthetic-secret\"",
            "https://synthetic:synthetic-secret@example.invalid/path",
            "ghp_synthetic-secret",
            "password: synthetic-secret",
            r#"{"api_key": "synthetic-secret"}"#,
            "X-API-Key: synthetic-secret",
            r#"{"host": "example.invalid", "api_key": "synthetic-secret"}"#,
            "MODE=synthetic API_KEY=synthetic-secret",
        ] {
            assert!(!redact(text).contains("synthetic-secret"));
        }
        assert_eq!(redact("Run cargo check"), "Run cargo check");
        assert_eq!(
            redact_record("Synthetic progress\r\n"),
            "Synthetic progress\r\n"
        );
    }

    #[test]
    fn saved_handoffs_are_exclusive_and_redacted() {
        let directory = tempfile::tempdir().unwrap();
        let handoff = Handoff {
            markdown: "Synthetic task\nexport API_KEY=synthetic-secret".into(),
            suggested_name: "synthetic.md".into(),
        };
        let path = save_in(&handoff, directory.path()).unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("synthetic-secret")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
