//! Keyboard handling. Digits/backspace edit the focused order-entry
//! field; letters are commands. Kept pure over (`App`, key, `conn`) so
//! a full session scripts cleanly in tests.

use crate::app::App;
use crate::conn::GatewayConn;
use crate::conn::Side;
use ratatui::crossterm::event::KeyCode;

/// What the event loop should do after a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Continue,
    Quit,
}

/// Apply one key. Digits go to the focused field, letters are
/// commands, Enter submits over `conn`.
pub fn handle_key(
    app: &mut App,
    code: KeyCode,
    conn: &mut dyn GatewayConn,
) -> Control {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Control::Quit,
        KeyCode::Char(c) if c.is_ascii_digit() => app.input_digit(c),
        KeyCode::Backspace => app.input_backspace(),
        KeyCode::Tab => app.toggle_focus(),
        KeyCode::Char('b') => app.set_side(Side::Buy),
        KeyCode::Char('s') => app.set_side(Side::Sell),
        KeyCode::Char('t') => app.cycle_tif(),
        KeyCode::Enter => {
            app.submit_order(conn);
        }
        // F3: toggle the trace HUD (diagnostic overlay).
        KeyCode::F(3) => app.show_trace = !app.show_trace,
        _ => {}
    }
    Control::Continue
}
