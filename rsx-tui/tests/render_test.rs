//! Integration tests: render the app into a `TestBackend` buffer and
//! assert on what actually reaches the screen. No real terminal.

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rsx_tui::App;
use rsx_tui::draw;

/// Flatten a rendered buffer into one string of its cell symbols so
/// tests can assert on visible text regardless of position.
fn render(app: &App, w: u16, h: u16) -> String {
    let mut terminal =
        Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal.draw(|f| draw(f, app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    buf.content().iter().map(|c| c.symbol()).collect()
}

#[test]
fn renders_symbol_and_demo_badge() {
    let s = render(&App::mock(), 120, 30);
    assert!(s.contains("PENGU-PERP"), "symbol in status bar");
    assert!(s.contains("DEMO"), "demo badge shown when disconnected");
}

#[test]
fn renders_book_panels_and_levels() {
    let s = render(&App::mock(), 120, 30);
    assert!(s.contains("book"), "book panel title");
    assert!(s.contains("spread"), "spread row");
    // best bid + best ask prices from the mock ladder.
    assert!(s.contains("10000"), "best bid price");
    assert!(s.contains("10001"), "best ask price");
}

#[test]
fn renders_positions_and_trades() {
    let s = render(&App::mock(), 120, 30);
    assert!(s.contains("positions"), "positions panel");
    assert!(s.contains("trades"), "trades panel");
    assert!(s.contains("upnl"), "positions header");
}

#[test]
fn spread_is_ask_minus_bid() {
    // mock: best ask 10001, best bid 10000 -> spread 1. Assert on the
    // computation (the rendered cell is width-clamped) + the label.
    let app = App::mock();
    assert_eq!(app.spread(), 1, "spread = best ask - best bid");
    let s = render(&app, 120, 30);
    assert!(s.contains("spread"), "spread row rendered");
}

#[test]
fn narrow_terminal_does_not_panic() {
    // Rendering into a tiny area must not panic (layout clamps).
    let _ = render(&App::mock(), 20, 8);
}
