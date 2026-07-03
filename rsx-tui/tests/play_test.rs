//! Play-like test: script a user session (a sequence of keystrokes),
//! re-render after each, and assert both the state transitions and
//! what the screen shows. This is the TUI analog of a Playwright flow
//! — deterministic, no real terminal or PTY.

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::Terminal;
use rsx_tui::App;
use rsx_tui::Control;
use rsx_tui::Side;
use rsx_tui::draw;
use rsx_tui::handle_key;

/// Drive one key and return the rendered screen text after it.
fn step(app: &mut App, code: KeyCode) -> (Control, String) {
    let ctrl = handle_key(app, code);
    let mut terminal =
        Terminal::new(TestBackend::new(120, 30)).expect("terminal");
    terminal.draw(|f| draw(f, app)).expect("draw");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    (ctrl, text)
}

#[test]
fn scripted_session_toggles_side_then_quits() {
    let mut app = App::mock();
    assert_eq!(app.side, Side::Buy, "starts on buy");

    // Press 's' -> side flips to Sell, loop continues.
    let (ctrl, _) = step(&mut app, KeyCode::Char('s'));
    assert_eq!(ctrl, Control::Continue);
    assert_eq!(app.side, Side::Sell);

    // Press 'b' -> back to Buy.
    let (ctrl, _) = step(&mut app, KeyCode::Char('b'));
    assert_eq!(ctrl, Control::Continue);
    assert_eq!(app.side, Side::Buy);

    // An unmapped key is a no-op (still Buy, still running).
    let (ctrl, _) = step(&mut app, KeyCode::Char('x'));
    assert_eq!(ctrl, Control::Continue);
    assert_eq!(app.side, Side::Buy);

    // 'q' quits.
    let (ctrl, _) = step(&mut app, KeyCode::Char('q'));
    assert_eq!(ctrl, Control::Quit);
}

#[test]
fn esc_also_quits() {
    let mut app = App::mock();
    let (ctrl, _) = step(&mut app, KeyCode::Esc);
    assert_eq!(ctrl, Control::Quit);
}

#[test]
fn book_panel_stays_rendered_across_the_session() {
    // Whatever the user presses, the ladder + panels keep rendering.
    let mut app = App::mock();
    for code in [
        KeyCode::Char('s'),
        KeyCode::Char('b'),
        KeyCode::Char('z'),
    ] {
        let (_, text) = step(&mut app, code);
        assert!(text.contains("book"), "book persists");
        assert!(text.contains("PENGU-PERP"), "symbol persists");
    }
}
