//! RSX trading terminal (ratatui) — binary entrypoint.
//!
//! Owns the terminal + event loop only; all UI state, rendering, and
//! key handling live in the library (`rsx_tui`) so they can be driven
//! from tests against a `TestBackend`. Run: `cargo run -p rsx-tui`.

use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::Terminal;
use rsx_tui::App;
use rsx_tui::Control;
use rsx_tui::draw;
use rsx_tui::handle_key;
use std::io;
use std::time::Duration;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

/// Event loop: redraw, then block up to 100ms for a key; `handle_key`
/// decides state changes and whether to quit.
fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
) -> io::Result<()> {
    let mut app = App::mock();
    loop {
        terminal.draw(|f| draw(f, &app))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if handle_key(&mut app, key.code) == Control::Quit {
                return Ok(());
            }
        }
    }
}
