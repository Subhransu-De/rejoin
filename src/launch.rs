use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::model::Agent;

#[derive(Clone, Debug)]
pub enum LaunchKind {
    Resume { agent: Agent, session_id: String },
    Handoff { target: Agent, markdown: String },
}

#[derive(Clone, Debug)]
pub struct LaunchRequest {
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

    let (agent, mut command) = build_command(request);
    command.current_dir(&request.cwd);

    // Paint the first frame before CreateProcess/exec so feedback is immediate.
    // The next operation is spawn: there is deliberately no startup sleep.
    let indicator = StartupIndicator::start(agent);
    let started = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", agent.binary()))?;
    if std::env::var_os("REJOIN_PROFILE").is_some() {
        eprintln!(
            "rejoin launch: spawned {} in {:.2} ms",
            agent.binary(),
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
    let status = child
        .wait()
        .with_context(|| format!("failed while waiting for {}", agent.binary()))?;
    indicator.stop();
    Ok(status)
}

struct StartupIndicator {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl StartupIndicator {
    fn start(agent: Agent) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&running);
        let frames = [".  ", ".. ", "..."];
        draw_opening(agent, frames[0], false);
        let initial_cursor = console_cursor();
        let worker = std::thread::spawn(move || {
            let mut frame = 0;
            let mut expected_cursor = initial_cursor;
            let mut fallback_ticks = 0_u8;

            while worker_running.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(90));
                if !worker_running.load(Ordering::Acquire) {
                    break;
                }

                // On Windows, inherited console output moves the cursor as soon
                // as the agent starts drawing. Stop before touching its UI.
                if let (Some(expected), Some(current)) = (expected_cursor, console_cursor())
                    && current != expected
                {
                    break;
                }

                // If cursor inspection is unavailable, keep the animation
                // bounded so it cannot interfere with an interactive agent.
                if expected_cursor.is_none() {
                    fallback_ticks = fallback_ticks.saturating_add(1);
                    if fallback_ticks >= 12 {
                        break;
                    }
                }

                frame = (frame + 1) % frames.len();
                draw_opening(agent, frames[frame], true);
                expected_cursor = console_cursor();
            }
        });
        Self {
            running,
            worker: Some(worker),
        }
    }

    fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for StartupIndicator {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn draw_opening(agent: Agent, dots: &str, update: bool) {
    let mut output = io::stdout().lock();
    if update {
        let _ = write!(
            output,
            "\x1b[1A\r\x1b[2Krejoin: opening {} {dots}\x1b[1B\r",
            agent.label()
        );
    } else {
        let _ = writeln!(output, "rejoin: opening {} {dots}", agent.label());
    }
    let _ = output.flush();
}

#[cfg(windows)]
fn console_cursor() -> Option<(i16, i16)> {
    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
    };

    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() {
        return None;
    }
    let mut info = std::mem::MaybeUninit::<CONSOLE_SCREEN_BUFFER_INFO>::uninit();
    if unsafe { GetConsoleScreenBufferInfo(handle, info.as_mut_ptr()) } == 0 {
        return None;
    }
    let cursor = unsafe { info.assume_init() }.dwCursorPosition;
    Some((cursor.X, cursor.Y))
}

#[cfg(not(windows))]
fn console_cursor() -> Option<(i16, i16)> {
    None
}

fn build_command(request: &LaunchRequest) -> (Agent, Command) {
    match &request.kind {
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
            let prompt = format!(
                "Continue this work from the agent-neutral handoff below. Verify the repository \
                 state first, then carry on with the remaining work.\n\n{markdown}"
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
    }
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
                kind: LaunchKind::Resume {
                    agent,
                    session_id: "session-id".to_owned(),
                },
                cwd: PathBuf::from("."),
            };
            let (_, command) = build_command(&request);
            let actual = command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "wrong resume arguments for {agent}");
        }
    }

    #[test]
    fn startup_indicator_does_not_delay_process_spawn() {
        let started = Instant::now();
        let indicator = StartupIndicator::start(Agent::Codex);
        #[cfg(windows)]
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/C", "exit", "0"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut child = Command::new("true").spawn().unwrap();
        let spawn_elapsed = started.elapsed();
        child.wait().unwrap();
        indicator.stop();
        eprintln!(
            "indicator plus process spawn: {:.2} ms",
            spawn_elapsed.as_secs_f64() * 1_000.0
        );
        assert!(
            spawn_elapsed < Duration::from_millis(250),
            "launch feedback delayed process creation by {spawn_elapsed:?}"
        );
    }
}
