//! Starts the program: checks that mpv is installed, then runs the key loop
//! and puts the terminal back when the program exits.

use std::io::{self, stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

mod app;
mod config;
mod deps;
mod ui;
mod visualizer;

use app::App;

struct TerminalGuard;

impl TerminalGuard {
    fn setup() -> Result<(Terminal<CrosstermBackend<io::Stdout>>, Self)> {
        let mut terminal = ratatui::init();
        execute!(stdout(), EnableBracketedPaste).context("enable bracketed paste")?;
        terminal.clear().context("clear terminal")?;
        Ok((terminal, Self))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

fn main() -> Result<()> {
    deps::ensure_mpv().context("checking for mpv")?;

    let (mut terminal, _guard) = TerminalGuard::setup().context("failed to set up terminal")?;
    let mut app = App::new();

    loop {
        app.tick();
        terminal
            .draw(|frame| ui::ui(frame, &mut app))
            .context("failed to draw")?;

        let timeout = if app.is_playing() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };

        if !event::poll(timeout).context("event poll")? {
            continue;
        }

        match event::read().context("event read")? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if app.handle_key(key) {
                    break;
                }
            }
            Event::Paste(text) => app.handle_paste(text),
            Event::Resize(_, _) => {}
            _ => {}
        }
    }

    app.stop();
    drop(_guard);
    println!("Thanks for using radio-tui!");
    Ok(())
}
