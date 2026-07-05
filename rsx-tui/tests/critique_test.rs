//! CEO/CTO critique findings, pinned as play tests. Each test names the
//! finding it guards. Same TestBackend harness as `play_test.rs`: build
//! an `App`, script keys / fold events, render into a `TestBackend`,
//! then assert on state and/or the rendered buffer — no real terminal
//! or network.

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::MockConn;
use rsx_tui::conn::Side;
use rsx_tui::drain;
use rsx_tui::draw;
use rsx_tui::handle_key;
use rsx_tui::GatewayConn;

fn type_str(app: &mut App, conn: &mut MockConn, s: &str) {
    for c in s.chars() {
        handle_key(app, KeyCode::Char(c), conn);
    }
}

fn screen(app: &App) -> String {
    let mut terminal =
        Terminal::new(TestBackend::new(120, 30)).expect("terminal");
    terminal.draw(|f| draw(f, app)).expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

// --- CEO: the F3 trace HUD (a headline diagnostic) must be both
// discoverable in the help bar and actually functional. ---

/// CEO finding: the F3 trace HUD was undiscoverable — the help bar
/// listed every other key (q/b/s/t/tab/digits/del/enter) but not F3, so
/// a fresh user had no way to learn the diagnostic overlay exists. The
/// help bar must advertise it.
#[test]
fn help_bar_advertises_the_f3_trace_hud() {
    let app = App::new("PENGU-PERP");
    let s = screen(&app);
    assert!(
        s.contains("F3"),
        "help bar must mention F3 so the trace HUD is discoverable",
    );
}

/// CEO finding: the F3 trace HUD is a whole feature (endpoint / link /
/// rtt readout) that had no test. Press F3 → the overlay appears with
/// its diagnostic rows; press F3 again → it hides. Pins the toggle and
/// the overlay content.
#[test]
fn f3_toggles_the_trace_hud_overlay() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    app.set_endpoint("ws://demo:8080");
    // Give it a connected link + one latency sample so the HUD has
    // something real to show.
    app.apply_event(GwEvent::Connected);
    app.apply_event(GwEvent::Latency {
        net_ns: Some(2_500),
        internal_ns: 7_600,
        engine_ns: 340,
    });

    assert!(!app.show_trace, "HUD hidden by default");
    assert!(!screen(&app).contains("TRACE"), "no overlay before F3");

    handle_key(&mut app, KeyCode::F(3), &mut conn);
    assert!(app.show_trace, "F3 flips show_trace on");
    let shown = screen(&app);
    assert!(shown.contains("TRACE"), "overlay title visible");
    assert!(shown.contains("endpoint"), "endpoint row visible");
    assert!(shown.contains("rtt p50"), "rtt row visible");
    assert!(shown.contains("ws://demo:8080"), "endpoint value shown");

    handle_key(&mut app, KeyCode::F(3), &mut conn);
    assert!(!app.show_trace, "F3 again flips it back off");
    assert!(!screen(&app).contains("TRACE"), "overlay gone after 2nd F3");
}

// --- CTO: order-entry form guards ---

/// CTO finding: a price that overflows i64 (typing a very long number)
/// makes `to_order` fail its parse, so the order is silently NOT sent.
/// The safety property — an unparseable value never reaches the gateway
/// — is what matters here and is pinned. (The status text reuses the
/// generic "incomplete order" message; the fields are non-empty, so the
/// message is imperfect, but the send is correctly suppressed.)
#[test]
fn overflowing_price_is_not_submitted() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    // 25 nines overflows i64 (max ~9.2e18, 19 digits).
    type_str(&mut app, &mut conn, &"9".repeat(25));
    handle_key(&mut app, KeyCode::Tab, &mut conn);
    type_str(&mut app, &mut conn, "5");
    handle_key(&mut app, KeyCode::Enter, &mut conn);
    assert!(
        conn.submitted.is_empty(),
        "an i64-overflowing price must never be sent to the gateway",
    );
}

/// CTO finding: an explicit zero price (parses fine, but `to_order`
/// rejects `price <= 0`) must not submit — a resting order at price 0 is
/// nonsense. Pins the `<= 0` guard at the form level (the existing
/// suite only covers a *missing* qty, not a typed-zero price).
#[test]
fn explicit_zero_price_is_not_submitted() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    type_str(&mut app, &mut conn, "0");
    handle_key(&mut app, KeyCode::Tab, &mut conn);
    type_str(&mut app, &mut conn, "5");
    handle_key(&mut app, KeyCode::Enter, &mut conn);
    assert!(conn.submitted.is_empty(), "price 0 must not be submitted");
    assert!(screen(&app).contains("incomplete"), "status explains why");
}

/// CTO finding: backspacing an already-empty field must not panic
/// (`String::pop` on empty is a no-op) — pins the empty-buffer edge.
#[test]
fn backspace_on_empty_field_is_a_noop() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    handle_key(&mut app, KeyCode::Backspace, &mut conn);
    assert!(app.entry.price.is_empty(), "still empty, no panic");
}

// --- CTO: state-fold bounds and invariants ---

/// CTO finding: `open_orders` is decremented with `saturating_sub`, so a
/// `Done` that arrives with no prior `Accepted` (a fully-filled IOC
/// emits FILLED→Done with no RESTING accept; or a duplicate Done) must
/// leave the count at 0, never underflow a `usize` to `usize::MAX`.
#[test]
fn done_without_accept_never_underflows_open_orders() {
    let mut app = App::new("PENGU-PERP");
    assert_eq!(app.open_orders, 0);
    app.apply_event(GwEvent::Done { oid: 1 });
    assert_eq!(app.open_orders, 0, "Done with no open order stays at 0");
    app.apply_event(GwEvent::Done { oid: 2 });
    assert_eq!(app.open_orders, 0, "second stray Done still 0, no wrap");
}

/// CTO finding: the trade tape is bounded (MAX_TRADES = 50) so a busy
/// session can't grow it without limit. Push 60 trades → exactly 50
/// retained, newest first (the last-pushed print at the front).
#[test]
fn trade_tape_is_bounded_and_newest_first() {
    let mut app = App::new("PENGU-PERP");
    for px in 0..60i64 {
        app.apply_event(GwEvent::Trade { side: Side::Buy, px, qty: 1 });
    }
    assert_eq!(app.trades.len(), 50, "tape capped at MAX_TRADES");
    assert_eq!(app.trades.front().map(|t| t.1), Some(59), "newest first");
    assert_eq!(app.trades.back().map(|t| t.1), Some(10), "oldest dropped");
}

/// CTO finding: the latency window is bounded (MAX_LAT = 128) so p50 /
/// best are computed over a rolling window, not an unbounded Vec. Fold
/// 140 complete samples → the window holds exactly 128.
#[test]
fn latency_window_is_bounded() {
    let mut app = App::new("PENGU-PERP");
    for i in 0..140u64 {
        app.apply_event(GwEvent::Latency {
            net_ns: Some(1_000 + i),
            internal_ns: 5_000,
            engine_ns: 200,
        });
    }
    assert_eq!(app.lat_totals.len(), 128, "window capped at MAX_LAT");
    assert!(app.lat_p50_ns().is_some(), "p50 available");
}

/// CTO finding: `apply_event` upserts positions by symbol — a second
/// `Position` for the same symbol updates the existing row in place
/// rather than appending a duplicate. Pins invariant "one row per
/// symbol" (a bug here would show the same market twice with stale pnl).
#[test]
fn position_update_upserts_by_symbol() {
    let mut app = App::new("PENGU-PERP");
    app.apply_event(GwEvent::Position {
        symbol: "PENGU-PERP".to_owned(),
        net_qty: 10,
        entry_px: 100,
        upnl: 5,
    });
    app.apply_event(GwEvent::Position {
        symbol: "PENGU-PERP".to_owned(),
        net_qty: 14,
        entry_px: 102,
        upnl: -3,
    });
    assert_eq!(app.positions.len(), 1, "same symbol upserts, no duplicate");
    let (_sym, net, entry, upnl) = &app.positions[0];
    assert_eq!((*net, *entry, *upnl), (14, 102, -3), "row updated in place");
}

/// CTO finding: `drain` folds every queued event in one pass (the
/// per-tick contract in `main.rs`), so a multi-event burst is fully
/// reflected after one drain — nothing is left in the transport.
#[test]
fn drain_folds_a_whole_burst_in_one_pass() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    conn.push_events(vec![
        GwEvent::Connected,
        GwEvent::Accepted { oid: 1 },
        GwEvent::Fill { oid: 1, px: 100, qty: 5, side: Side::Buy },
        GwEvent::Done { oid: 1 },
    ]);
    drain(&mut app, &mut conn);
    assert!(app.connected);
    assert_eq!(app.fills, 1);
    assert_eq!(app.open_orders, 0, "Accepted then Done nets to 0");
    assert!(conn.poll_event().is_none(), "transport fully drained");
}
