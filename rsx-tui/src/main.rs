//! RSX trading terminal (ratatui) — binary entrypoint.
//!
//! Owns the terminal + event loop only. State, rendering, input, and
//! the gateway transport live in the `rsx_tui` library so they run
//! under a `TestBackend` in tests. Until the QUIC client lands this
//! drives a `MockConn` seeded with a demo feed. Run: `cargo run -p
//! rsx-tui`.

use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::conn::MockConn;
use rsx_tui::demo_events;
use rsx_tui::drain;
use rsx_tui::draw;
use rsx_tui::handle_key;
use rsx_tui::Control;
use std::io;
use std::time::Duration;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

/// Event loop: drain gateway events, redraw, then block up to 100ms
/// for a key. `handle_key` mutates state / submits and says when to
/// quit.
fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
) -> io::Result<()> {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    conn.push_events(demo_events());

    loop {
        drain(&mut app, &mut conn);
        terminal.draw(|f| draw(f, &app))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if handle_key(&mut app, key.code, &mut conn) == Control::Quit {
                return Ok(());
            }
        }
    }
}
