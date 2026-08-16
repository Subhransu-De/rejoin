use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

fn rejoin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rejoin"))
}

fn configured_rejoin(root: &Path) -> Command {
    let mut command = rejoin();
    command
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("LOCALAPPDATA", root.join("cache"))
        .arg("--claude-home")
        .arg(root.join("claude"))
        .arg("--codex-home")
        .arg(root.join("codex"))
        .arg("--cursor-home")
        .arg(root.join("cursor"))
        .arg("--pi-session-dir")
        .arg(root.join("pi"))
        .arg("--opencode-database")
        .arg(root.join("opencode.db"))
        .arg("--all");
    command
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn help_and_version_are_available() {
    let help = rejoin().arg("--help").output().unwrap();
    assert_success(&help);
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Unified coding-agent session manager"));
    assert!(help.contains("list"));
    assert!(help.contains("paths"));

    let version = rejoin().arg("--version").output().unwrap();
    assert_success(&version);
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("rejoin {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn paths_reports_every_override_and_all_folders_scope() {
    let directory = TempDir::new().unwrap();
    let output = configured_rejoin(directory.path())
        .arg("paths")
        .output()
        .unwrap();
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();

    for expected in [
        directory.path().join("claude").join("projects"),
        directory.path().join("codex").join("sessions"),
        directory.path().join("cursor").join("chats"),
        directory.path().join("pi"),
        directory.path().join("opencode.db"),
    ] {
        assert!(
            stdout.contains(&expected.display().to_string()),
            "missing {} in:\n{stdout}",
            expected.display()
        );
    }
    assert!(stdout.contains("Scope    all folders"));
}

#[test]
fn list_json_has_a_stable_empty_result() {
    let directory = TempDir::new().unwrap();
    let output = configured_rejoin(directory.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!([])
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn corrupt_opencode_store_is_reported_without_losing_json_output() {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join("opencode.db"),
        b"not a sqlite database",
    )
    .unwrap();
    let output = configured_rejoin(directory.path())
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!([])
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("warning: OpenCode:"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn tui_rejects_noninteractive_input_with_a_script_hint() {
    let directory = TempDir::new().unwrap();
    let output = configured_rejoin(directory.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("requires an interactive terminal"));
    assert!(stderr.contains("rejoin list --json"));
}
