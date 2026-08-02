mod app;
mod handoff;
mod launch;
mod model;
mod scanner;
mod ui;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use app::{App, AppAction};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use scanner::ScanOptions;

#[derive(Debug, Parser)]
#[command(
    name = "rejoin",
    version,
    about = "Unified coding-agent session manager"
)]
struct Cli {
    /// Override the Claude Code data directory.
    #[arg(long, global = true, value_name = "DIR")]
    claude_home: Option<PathBuf>,

    /// Override the Codex data directory.
    #[arg(long, global = true, value_name = "DIR")]
    codex_home: Option<PathBuf>,

    /// Override the Cursor Agent data directory.
    #[arg(long, global = true, value_name = "DIR")]
    cursor_home: Option<PathBuf>,

    /// Override Pi's session directory.
    #[arg(long, global = true, value_name = "DIR")]
    pi_session_dir: Option<PathBuf>,

    /// Override the OpenCode SQLite database.
    #[arg(long, global = true, value_name = "FILE")]
    opencode_database: Option<PathBuf>,

    /// Show sessions from every working directory instead of only this folder.
    #[arg(long, global = true)]
    all: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List discovered sessions without opening the TUI.
    List {
        /// Emit JSON for scripts and diagnostics.
        #[arg(long)]
        json: bool,
    },
    /// Print the resolved session-store paths and folder scope.
    Paths,
}

type Tui = Terminal<CrosstermBackend<io::Stdout>>;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let options = ScanOptions::discover(
        cli.claude_home,
        cli.codex_home,
        cli.cursor_home,
        cli.pi_session_dir,
        cli.opencode_database,
        cli.all,
    )?;
    match cli.command {
        Some(Command::List { json }) => list_sessions(&options, json),
        Some(Command::Paths) => {
            print_paths(&options);
            Ok(())
        }
        None => run_tui(options),
    }
}

fn print_paths(options: &ScanOptions) {
    println!(
        "Claude   {}",
        options.claude_home.join("projects").display()
    );
    println!("Codex    {}", options.codex_home.join("sessions").display());
    println!("Cursor   {}", options.cursor_home.join("chats").display());
    println!("Pi       {}", options.pi_sessions.display());
    println!("OpenCode {}", options.opencode_database.display());
    println!(
        "Scope    {}",
        options
            .scope
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "all folders".to_owned())
    );
}

fn list_sessions(options: &ScanOptions, json: bool) -> Result<()> {
    let result = scanner::scan(options);
    if json {
        serde_json::to_writer_pretty(io::stdout(), &result.sessions)?;
        println!();
    } else {
        println!(
            "{:<8} {:<8} {:<20} {:<48} ACTIVITY",
            "STATUS", "AGENT", "PROJECT", "SESSION"
        );
        for session in &result.sessions {
            println!(
                "{:<8} {:<8} {:<20.20} {:<48.48} {}",
                session.status.label(),
                session.agent.label(),
                session.project,
                session.title,
                model::relative_time(session.last_activity),
            );
        }
    }
    for warning in result.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn run_tui(options: ScanOptions) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("rejoin requires an interactive terminal; use `rejoin list --json` for scripts");
    }

    install_panic_hook();
    let mut terminal = enter_terminal()?;
    let mut app = App::load(options);

    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            app.tick();
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            match app.handle_key(key) {
                AppAction::None => {}
                AppAction::Quit => break,
                AppAction::Launch(request) => {
                    prepare_terminal_for_agent(&mut terminal)?;
                    let status = launch::execute(&request)?;
                    if !status.success() {
                        bail!("agent exited with status {status}");
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    })();

    leave_terminal(&mut terminal)?;
    result
}

fn prepare_terminal_for_agent(terminal: &mut Tui) -> Result<()> {
    // Keep rejoin's alternate screen active while the child owns the terminal.
    // Inline TUIs can then render freely without polluting the shell's primary
    // buffer; leave_terminal restores that buffer after the agent exits.
    disable_raw_mode().context("could not disable terminal raw mode")?;
    terminal.clear()?;
    terminal.show_cursor()?;
    Ok(())
}

fn enter_terminal() -> Result<Tui> {
    enable_raw_mode().context("could not enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("could not enter alternate screen")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    Ok(terminal)
}

fn leave_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode().context("could not disable terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("could not leave alternate screen")?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}
