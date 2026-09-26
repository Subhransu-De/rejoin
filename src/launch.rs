use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use crate::model::Agent;

#[derive(Clone, Debug)]
pub enum LaunchKind {
    New { agent: Agent },
    Resume { agent: Agent, session_id: String },
    Handoff { target: Agent, markdown: String },
}

#[derive(Clone, Debug)]
pub struct LaunchRequest {
    #[cfg(windows)]
    pub cursor_home: PathBuf,
    pub kind: LaunchKind,
    pub cwd: PathBuf,
}

pub fn execute(request: &LaunchRequest) -> Result<ExitStatus> {
    if !request.cwd.is_dir() {
        bail!(
            "working directory does not exist: {}",
            request.cwd.display()
        );
    }
    let (agent, command) = build_command(request)?;
    let mut command = command;
    #[cfg(windows)]
    if agent == Agent::Cursor
        && let Some((distro, _)) = crate::scanner::wsl_path(&request.cursor_home)
            .or_else(|| crate::scanner::wsl_path(&request.cwd))
    {
        let cwd = windows_path_for_wsl(&request.cwd, &distro)?;
        let args = command
            .get_args()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        command = Command::new("wsl.exe");
        command
            .args([
                "--distribution",
                &distro,
                "--cd",
                &cwd,
                "--",
                "cursor-agent",
            ])
            .args(args);
    }
    if command.get_program() != "wsl.exe" {
        command = resolve_command(command);
        command.current_dir(&request.cwd);
    }
    println!("Opening {}…", agent.label());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).with_context(|| format!("failed to start {}", agent.binary()))
    }
    #[cfg(windows)]
    {
        let _guard = ConsoleInterruptGuard::install()?;
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to start {}", agent.binary()))?;
        child
            .wait()
            .with_context(|| format!("failed while waiting for {}", agent.binary()))
    }
}

fn resolve_command(command: Command) -> Command {
    #[cfg(windows)]
    {
        let program = command.get_program().to_string_lossy();
        for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            for extension in ["exe", "cmd", "bat", "ps1"] {
                let path = directory.join(format!("{program}.{extension}"));
                if path.is_file() {
                    let mut resolved = if extension == "ps1" {
                        let mut host = Command::new("powershell.exe");
                        host.args(["-NoLogo", "-NoProfile", "-File"]).arg(path);
                        host
                    } else {
                        Command::new(path)
                    };
                    resolved.args(command.get_args());
                    return resolved;
                }
            }
        }
    }
    command
}

#[cfg(windows)]
struct ConsoleInterruptGuard;
#[cfg(windows)]
unsafe extern "system" fn handle_interrupt(event: u32) -> i32 {
    i32::from(
        event == windows_sys::Win32::System::Console::CTRL_C_EVENT
            || event == windows_sys::Win32::System::Console::CTRL_BREAK_EVENT,
    )
}
#[cfg(windows)]
impl ConsoleInterruptGuard {
    fn install() -> Result<Self> {
        // SAFETY: the callback has the required ABI and remains valid for the process lifetime.
        if unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(handle_interrupt), 1)
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self)
    }
}
#[cfg(windows)]
impl Drop for ConsoleInterruptGuard {
    fn drop(&mut self) {
        // SAFETY: this removes the same process-local handler installed by this guard.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(handle_interrupt), 0);
        }
    }
}

fn build_command(request: &LaunchRequest) -> Result<(Agent, Command)> {
    Ok(match &request.kind {
        LaunchKind::New { agent } => (*agent, Command::new(agent.binary())),
        LaunchKind::Resume { agent, session_id } => {
            let mut command = Command::new(agent.binary());
            match agent {
                Agent::Claude => {
                    command.args(["--resume", session_id]);
                }
                Agent::Codex => {
                    command.args(["resume", session_id]);
                }
                Agent::Cursor => {
                    command.args(["--resume", session_id]);
                }
                Agent::Pi => {
                    command.args(["--session", session_id]);
                }
                Agent::OpenCode => {
                    command.args(["--session", session_id]);
                }
            }
            (*agent, command)
        }
        LaunchKind::Handoff { target, markdown } => {
            let file = crate::handoff::save(
                &crate::model::Handoff {
                    markdown: markdown.clone(),
                    suggested_name: "handoff.md".to_owned(),
                },
                &request.cwd,
            )?;
            let reference = file.display().to_string();
            #[cfg(windows)]
            let reference = if *target == Agent::Cursor {
                if let Some((distro, _)) = crate::scanner::wsl_path(&request.cursor_home)
                    .or_else(|| crate::scanner::wsl_path(&request.cwd))
                {
                    windows_path_for_wsl(&file, &distro)?
                } else {
                    reference
                }
            } else {
                reference
            };
            let prompt = format!(
                "Read the handoff at {} first. Verify the repository state, then continue the remaining work.",
                reference
            );
            let mut command = Command::new(target.binary());
            match target {
                Agent::Codex => {
                    command.arg("-C").arg(&request.cwd).arg(prompt);
                }
                Agent::OpenCode => {
                    command.arg("--prompt").arg(prompt);
                }
                Agent::Claude | Agent::Cursor | Agent::Pi => {
                    command.arg(prompt);
                }
            }
            (*target, command)
        }
    })
}

#[cfg(windows)]
fn windows_path_for_wsl(path: &std::path::Path, distro: &str) -> Result<String> {
    let path = std::path::absolute(path)?;
    let text = path.to_string_lossy();
    let text = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        text.trim_start_matches(r"\\?\").to_owned()
    };
    if let Some((found, linux)) = crate::scanner::wsl_path(std::path::Path::new(&text)) {
        if found != distro {
            bail!("workspace belongs to a different WSL distribution");
        }
        return Ok(linux);
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Ok(format!(
            "/mnt/{}/{}",
            (bytes[0] as char).to_ascii_lowercase(),
            text[3..].replace('\\', "/")
        ));
    }
    bail!("path cannot be translated for WSL: {}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_arguments_match_each_agent_cli() {
        let cases = [
            (Agent::Claude, vec!["--resume", "session-id"]),
            (Agent::Codex, vec!["resume", "session-id"]),
            (Agent::Cursor, vec!["--resume", "session-id"]),
            (Agent::Pi, vec!["--session", "session-id"]),
            (Agent::OpenCode, vec!["--session", "session-id"]),
        ];
        for (agent, expected) in cases {
            let request = LaunchRequest {
                #[cfg(windows)]
                cursor_home: PathBuf::new(),
                kind: LaunchKind::Resume {
                    agent,
                    session_id: "session-id".to_owned(),
                },
                cwd: PathBuf::from("."),
            };
            let (_, command) = build_command(&request).unwrap();
            let actual = command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "wrong resume arguments for {agent}");
        }
    }

    #[test]
    fn new_session_starts_the_selected_agent_without_resume_arguments() {
        for agent in Agent::ALL {
            let request = LaunchRequest {
                #[cfg(windows)]
                cursor_home: PathBuf::new(),
                kind: LaunchKind::New { agent },
                cwd: PathBuf::from("."),
            };
            let (actual_agent, command) = build_command(&request).unwrap();

            assert_eq!(actual_agent, agent);
            assert_eq!(command.get_program(), agent.binary());
            assert_eq!(command.get_args().count(), 0);
        }

        #[cfg(windows)]
        assert_eq!(Agent::Cursor.binary(), "cursor-agent");
    }

    #[test]
    #[cfg(windows)]
    fn windows_batch_shim_accepts_a_short_file_prompt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.cmd");
        std::fs::write(&path, "@echo off\r\nexit /b 23\r\n").unwrap();
        let status = Command::new(path)
            .arg("Read the handoff file first.")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(23));
    }

    #[test]
    #[cfg(windows)]
    fn translates_drive_and_distribution_paths_for_wsl() {
        assert_eq!(
            windows_path_for_wsl(
                std::path::Path::new(r"C:\synthetic\handoff.md"),
                "SyntheticDistro"
            )
            .unwrap(),
            "/mnt/c/synthetic/handoff.md"
        );
        assert_eq!(
            windows_path_for_wsl(
                std::path::Path::new(r"\\wsl.localhost\SyntheticDistro\home\synthetic"),
                "SyntheticDistro"
            )
            .unwrap(),
            "/home/synthetic"
        );
        assert!(
            windows_path_for_wsl(
                std::path::Path::new(r"\\wsl.localhost\OtherDistro\home\synthetic"),
                "SyntheticDistro"
            )
            .is_err()
        );
    }
}
