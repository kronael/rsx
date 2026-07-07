//! Proves `support::harness::TuiHarness` works with no cluster: drives
//! a scripted session over `MockConn` (type an order, submit it, fold
//! a scripted `Fill`) and asserts both state and screen. T3-T5 build
//! their cluster-gated tests on this same harness; this is the
//! offline half that must pass with nothing running.

mod support;

use ratatui::crossterm::event::KeyCode;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::Side;
use rsx_tui::Control;
use std::time::Duration;
use support::harness::TuiHarness;

#[test]
fn scripted_session_types_submits_and_folds_a_fill() {
    let mut harness = TuiHarness::new_mock();
    harness.assert_screen("offline");

    // Buy 5 @ 10_001, GTC (default tif), submit.
    let ctrl = harness.feed_key(KeyCode::Char('b'));
    assert_eq!(ctrl, Control::Continue);
    harness.feed_str("10001");
    harness.feed_key(KeyCode::Tab);
    harness.feed_str("5");
    let ctrl = harness.feed_key(KeyCode::Enter);
    assert_eq!(ctrl, Control::Continue);

    harness.assert_state("order sent, form cleared", |app| {
        app.entry.price.is_empty() && app.entry.qty.is_empty()
    });
    harness.assert_screen("sent");

    // Fold a scripted Accepted -> Fill -> Done from the MockConn the
    // way a real gateway ack sequence would arrive, then wait_for
    // observes it the same way a cluster e2e test waits on a real
    // WsConn.
    harness.push_mock_events([
        GwEvent::Accepted { oid: 1 },
        GwEvent::Fill {
            oid: 1,
            px: 10_001,
            qty: 5,
            side: Side::Buy,
        },
        GwEvent::Done { oid: 1 },
    ]);

    let waited = harness
        .wait_for(|app| app.fills == 1, Duration::from_secs(1))
        .expect("Fill observed within timeout");
    assert!(
        waited < Duration::from_secs(1),
        "wait_for reports elapsed time"
    );

    harness.assert_state("fill folded", |app| app.fills == 1);
    harness.assert_state("order lifecycle closed", |app| app.open_orders == 0);
    // Status bar's "fills N" counter (Done overwrites the status line's
    // free-text message, but the counter in the top bar stays put).
    harness.assert_screen("fills 1");
}
