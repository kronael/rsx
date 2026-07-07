//! Integration tests: fold the demo feed into the app, render into a
//! `TestBackend` buffer, and assert on what actually reaches the
//! screen. No real terminal.

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::demo_events;
use rsx_tui::draw;

/// Build a connected app from the demo feed.
fn demo_app() -> App {
    let mut app = App::new("PENGU-PERP");
    for ev in demo_events() {
        app.apply_event(ev);
    }
    app
}

/// Flatten a rendered buffer into one string of cell symbols.
fn render(app: &App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal.draw(|f| draw(f, app)).expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn renders_symbol_and_live_badge() {
    let s = render(&demo_app(), 120, 30);
    assert!(s.contains("PENGU-PERP"), "symbol in status bar");
    assert!(s.contains("live"), "live badge once Connected folded in");
}

#[test]
fn renders_book_panels_and_levels() {
    let s = render(&demo_app(), 120, 30);
    assert!(s.contains("book"), "book panel title");
    assert!(s.contains("10000"), "best bid price from demo feed");
    assert!(s.contains("10001"), "best ask price from demo feed");
}

#[test]
fn renders_order_entry_and_panels() {
    let s = render(&demo_app(), 120, 30);
    assert!(s.contains("order"), "order-entry panel");
    assert!(s.contains("BUY"), "buy button");
    assert!(s.contains("GTC"), "default tif shown");
    assert!(s.contains("positions"), "positions panel");
    assert!(s.contains("trades"), "trades panel");
}

#[test]
fn spread_is_ask_minus_bid() {
    // demo: best ask 10001, best bid 10000 -> spread 1.
    assert_eq!(demo_app().spread(), 1);
}

#[test]
fn empty_app_offline_before_connect() {
    let s = render(&App::new("PENGU-PERP"), 120, 30);
    assert!(s.contains("offline"), "offline until Connected event");
}

#[test]
fn renders_speed_strip_breakdown() {
    // demo feed includes Latency events, so the speed strip shows the
    // net/internal/engine split and formatted values.
    let s = render(&demo_app(), 120, 30);
    assert!(s.contains("RTT"), "speed strip present");
    assert!(s.contains("net"), "net leg labelled");
    assert!(s.contains("internal"), "internal leg labelled");
    assert!(s.contains("engine"), "engine leg labelled");
    assert!(s.contains("ns") || s.contains("µs"), "formatted latency");
}

#[test]
fn speed_strip_waits_before_first_measurement() {
    let s = render(&App::new("PENGU-PERP"), 120, 30);
    assert!(s.contains("waiting"), "speed strip idle before first RTT");
}

#[test]
fn narrow_terminal_does_not_panic() {
    let _ = render(&demo_app(), 20, 8);
}
