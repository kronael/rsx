//! Play-like tests: script a full user session (keystrokes + gateway
//! events) and assert both state transitions and what the screen
//! shows. The TUI analog of a Playwright flow — deterministic, no real
//! terminal or network (drives a `MockConn`).

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::app::Latency;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::MockConn;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::demo_events;
use rsx_tui::drain;
use rsx_tui::draw;
use rsx_tui::handle_key;
use rsx_tui::Control;

/// Type a string of digits key by key.
fn type_digits(app: &mut App, conn: &mut MockConn, s: &str) {
    for c in s.chars() {
        handle_key(app, KeyCode::Char(c), conn);
    }
}

fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("terminal");
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
fn drains_gateway_events_into_state() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    conn.push_events(demo_events());
    drain(&mut app, &mut conn);
    assert!(app.connected, "Connected folded in");
    assert_eq!(app.bids.first(), Some(&(10_000, 7)));
    assert_eq!(app.asks.first(), Some(&(10_001, 5)));
    assert_eq!(app.trades.len(), 2);
    assert_eq!(app.positions.len(), 1);
}

#[test]
fn types_and_submits_a_sell_ioc_order() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();

    // Sell side, price 10002, tab to qty, qty 3, tif -> IOC, submit.
    handle_key(&mut app, KeyCode::Char('s'), &mut conn);
    type_digits(&mut app, &mut conn, "10002");
    handle_key(&mut app, KeyCode::Tab, &mut conn);
    type_digits(&mut app, &mut conn, "3");
    handle_key(&mut app, KeyCode::Char('t'), &mut conn); // GTC->IOC
    let ctrl = handle_key(&mut app, KeyCode::Enter, &mut conn);

    assert_eq!(ctrl, Control::Continue);
    assert_eq!(conn.submitted.len(), 1, "one order sent");
    let o = conn.submitted[0];
    assert_eq!(o.side, Side::Sell);
    assert_eq!(o.price, 10_002);
    assert_eq!(o.qty, 3);
    assert_eq!(o.tif, Tif::Ioc);
    // Form clears after a successful submit.
    assert!(app.entry.price.is_empty());
    assert!(app.entry.qty.is_empty());
    assert!(screen(&app).contains("sent"), "status confirms send");
}

#[test]
fn backspace_edits_the_focused_field() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    type_digits(&mut app, &mut conn, "199");
    handle_key(&mut app, KeyCode::Backspace, &mut conn);
    assert_eq!(app.entry.price, "19");
}

#[test]
fn incomplete_order_is_not_sent() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    // price only, no qty.
    type_digits(&mut app, &mut conn, "100");
    handle_key(&mut app, KeyCode::Enter, &mut conn);
    assert!(conn.submitted.is_empty(), "no qty -> nothing sent");
    assert!(screen(&app).contains("incomplete"), "status says why");
}

#[test]
fn submit_on_down_link_reports_failure() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    conn.down = true;
    type_digits(&mut app, &mut conn, "100");
    handle_key(&mut app, KeyCode::Tab, &mut conn);
    type_digits(&mut app, &mut conn, "5");
    handle_key(&mut app, KeyCode::Enter, &mut conn);
    assert!(conn.submitted.is_empty(), "link down -> not sent");
    assert!(screen(&app).contains("failed"), "status reports failure");
}

#[test]
fn q_and_esc_quit() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    assert_eq!(
        handle_key(&mut app, KeyCode::Char('q'), &mut conn),
        Control::Quit,
    );
    assert_eq!(handle_key(&mut app, KeyCode::Esc, &mut conn), Control::Quit,);
}

#[test]
fn latency_events_fold_into_stats() {
    let mut app = App::new("PENGU-PERP");
    app.apply_event(GwEvent::Latency {
        net_ns: Some(2_500),
        internal_ns: 7_600,
        engine_ns: 340,
    });
    app.apply_event(GwEvent::Latency {
        net_ns: Some(2_300),
        internal_ns: 7_100,
        engine_ns: 310,
    });
    assert_eq!(
        app.last_lat,
        Some(Latency {
            net_ns: Some(2_300),
            internal_ns: 7_100,
            engine_ns: 310,
        }),
    );
    // totals: 10_440 then 9_710 -> sorted [9_710, 10_440].
    assert_eq!(app.lat_p50_ns(), Some(10_440));
    assert_eq!(app.lat_min_ns(), Some(9_710));

    // An incomplete sample (net unmeasured) updates `last` for display
    // but must NOT enter the p50 / best window.
    app.apply_event(GwEvent::Latency {
        net_ns: None,
        internal_ns: 5_000,
        engine_ns: 200,
    });
    assert_eq!(app.last_lat.map(|l| l.net_ns), Some(None));
    assert_eq!(app.lat_min_ns(), Some(9_710), "incomplete sample excluded");
}

#[test]
fn side_and_tif_toggle() {
    let mut app = App::new("PENGU-PERP");
    let mut conn = MockConn::new();
    handle_key(&mut app, KeyCode::Char('s'), &mut conn);
    assert_eq!(app.entry.side, Side::Sell);
    handle_key(&mut app, KeyCode::Char('b'), &mut conn);
    assert_eq!(app.entry.side, Side::Buy);
    assert_eq!(app.entry.tif, Tif::Gtc);
    handle_key(&mut app, KeyCode::Char('t'), &mut conn);
    assert_eq!(app.entry.tif, Tif::Ioc);
}
