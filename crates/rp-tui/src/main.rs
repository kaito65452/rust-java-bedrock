#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

#[cfg(not(any(target_os = "linux", target_os = "android")))]
compile_error!("rp-tui is built exclusively for Linux terminals (testing/management edition).");

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use rp_cli::load_mappings;
use rp_tui::app::{ActiveTab, App};
use rp_tui::ui;

#[derive(Debug, Parser)]
#[command(
    name = "rp-tui",
    version,
    about = "Linux Terminal UI for managing, inspecting, converting, and auditing Minecraft Java <-> Bedrock resource packs"
)]
struct TuiArgs {
    /// Directory to scan for .zip / .mcpack / unpacked resource packs (defaults to current directory).
    #[arg(default_value = ".")]
    dir: PathBuf,
    /// Optional external JSON mapping table override file.
    #[arg(long = "mapping")]
    mapping: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = TuiArgs::parse();
    let scan_dir = args.dir.canonicalize().unwrap_or(args.dir);
    let mappings = load_mappings(args.mapping.as_deref())?;

    let mut app = App::new(scan_dir, args.mapping, mappings);
    app.start_scan();

    // Restore terminal cleanly even if anything unwinds unexpectedly.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        orig_hook(info);
    }));

    enable_raw_mode().context("failed to enable Linux terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize ratatui terminal")?;

    let run_res = run_event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    run_res
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    while !app.should_quit {
        app.poll_worker();
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(mouse) => {
                    if let MouseEventKind::Down(_) = mouse.kind
                        && mouse.row <= 2
                    {
                        // Allow clicking tabs in the header bar.
                        let idx = (mouse.column as usize) / 16;
                        if let Some(&tab) = ActiveTab::ALL.get(idx) {
                            app.active_tab = tab;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
