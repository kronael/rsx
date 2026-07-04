//! Order lifecycle e2e (T3): submit/fill/reject over a real gateway
//! connection (`WsConn`), driven through `TuiHarness`. Env-gated —
//! `skip_if_no_cluster!` skips cleanly when nothing is listening at
//! `RSX_GW_LISTEN` (start one with
//! `./rsx-playground/playground start-all minimal`).
//!
//! Each test uses its own price band so tests sharing the one live
//! order book don't cross each other: `submit_gtc_rests` @ 1 (far below
//! the demo symbol's ~50_000-60_000 trading range, so no leftover
//! resting ask from prior runs/benches crosses it — this book is
//! shared, long-lived state, not reset per test), `submit_ioc_fills`
//! maker @ 60_000 / taker @ 59_000, `order_lifecycle_accepted_then_done`
//! maker @ 58_000 / taker @ 57_000. `invalid_order_rejected` sends a
//! malformed order directly (bypassing the entry form, which would
//! never send it) so the real gateway's own validation rejects it.
//!
//! user_ids are drawn from the playground's seeded demo accounts (`1,
//! 2, 3, 4, 5, 99` — `rsx-playground/server.py`'s `_SEED_USERS`, the
//! only accounts with collateral loaded into `rsx-risk`'s in-memory
//! shard at replay time). A never-seeded user_id gets a fresh
//! zero-collateral account and every non-trivial order is silently
//! `InsufficientMargin`-rejected — no `Accepted`, so `wait_for` just
//! times out looking like a hang, not a rejection. Confirmed against
//! rsx-risk/src/shard.rs `ensure_account`/`check_order`.
//!
//! Casting is UDP: `rsx-matching` logs occasional `cmp receiver
//! FAULTED: ... skipping unrecoverable order gap ... clients re-send
//! dropped pre-ack orders (WAL dedup = exactly-once)` — an order (or
//! its return-leg fill/done event) can be dropped in flight, by design,
//! with the sender expected to notice the missing ack and resubmit.
//! `submit_and_wait`/`submit_and_record` below do exactly one such
//! resubmit (a fresh cid, so it's a new attempt, not a WAL-deduped
//! replay) before treating a timeout as a real failure.

mod support;

use ratatui::crossterm::event::KeyCode;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::OrderReq;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::App;
use rsx_tui::Control;
use rsx_tui::GatewayConn;
use rsx_tui::WsConn;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use support::cluster;
use support::harness::TuiHarness;

/// Up to this many submit attempts before giving up — see the module
/// doc's casting-loss note. Observed empirically: the very first order
/// on a fresh connection occasionally needs a resubmit before its ack
/// arrives (manually reproduced over a raw WS session outside this
/// suite: a first submission got no reply within 2s, an identical
/// resubmit on a new connection was acked immediately). See BUGS.md
/// for the open item tracking the underlying return-path flakiness —
/// this retry is a test-side accommodation, not a fix.
const SUBMIT_ATTEMPTS: u32 = 2;

/// Submit `order` over `harness.conn` and wait for `pred`, resubmitting
/// (a fresh cid each time, so never a WAL-deduped replay) up to
/// `SUBMIT_ATTEMPTS` times total before treating it as a real failure.
fn submit_and_wait<F>(
    harness: &mut TuiHarness,
    order: OrderReq,
    pred: F,
    timeout: Duration,
) -> Duration
where
    F: Fn(&App) -> bool,
{
    for attempt in 1..=SUBMIT_ATTEMPTS {
        harness.conn.submit(order).expect("submit order");
        if let Some(elapsed) = harness.wait_for(&pred, timeout) {
            return elapsed;
        }
        eprintln!(
            "attempt {attempt}/{SUBMIT_ATTEMPTS}: no ack within {timeout:?}; \
             casting is UDP, an occasional dropped order/event is expected \
             — see rsx-matching's FAULTED gap log",
        );
    }
    panic!("order never acked after {SUBMIT_ATTEMPTS} attempts");
}

/// A resting GTC buy at a price band no other test crosses: `Accepted`
/// folds into `App.open_orders`.
///
/// Does not assert `app.connected` — `cluster::connect` already polls
/// for and consumes the one `GwEvent::Connected` this socket will ever
/// produce (that's how it knows the handshake succeeded), so by the
/// time `TuiHarness::new_with` builds an `App` around the returned
/// `WsConn`, that event is gone and `app.connected` never flips true.
#[test]
fn submit_gtc_rests() {
    let conn = skip_if_no_cluster!(cluster::connect(1));
    let mut harness = TuiHarness::new_with(Box::new(conn));

    harness.feed_str("1");
    harness.feed_key(KeyCode::Tab);
    harness.feed_str("100000");
    let ctrl = harness.feed_key(KeyCode::Enter);
    assert_eq!(ctrl, Control::Continue);

    harness
        .wait_for(|app| app.open_orders == 1, Duration::from_secs(5))
        .expect("Accepted observed within timeout");
    harness.assert_state("order rests", |app| app.open_orders == 1);
    harness.assert_screen("open 1");
}

/// Seed a resting maker, then submit a crossing IOC that fully fills
/// it: `Fill` then `Done`, `fills==1`/`open_orders==0`.
#[test]
#[ignore = "blocked on RETURN-PATH-INTERMITTENT-DROP (bugs.md): taker fill leg \
            drops on the persistent WsConn path; run with --ignored once fixed"]
fn submit_ioc_fills() {
    let mut maker = skip_if_no_cluster!(cluster::connect(2));
    cluster::seed_book(&mut maker);

    let conn = skip_if_no_cluster!(cluster::connect(3));
    let mut harness = TuiHarness::new_with(Box::new(conn));
    submit_and_wait(
        &mut harness,
        OrderReq { side: Side::Sell, price: 59_000, qty: 500_000, tif: Tif::Ioc },
        |app| app.fills == 1 && app.open_orders == 0,
        Duration::from_secs(5),
    );

    harness.assert_state("fill folded", |app| app.fills == 1);
    harness.assert_state("no open orders after full IOC fill", |app| {
        app.open_orders == 0
    });
    harness.assert_screen("fills 1");
}

/// A malformed order (`qty=0`) sent directly on the connection —
/// `OrderEntry::to_order` would never send this, so this exercises the
/// real gateway's own validation, not the TUI's form guard.
#[test]
fn invalid_order_rejected() {
    let conn = skip_if_no_cluster!(cluster::connect(4));
    let mut harness = TuiHarness::new_with(Box::new(conn));
    submit_and_wait(
        &mut harness,
        OrderReq { side: Side::Buy, price: 12_345, qty: 0, tif: Tif::Gtc },
        |app| app.status.contains("rejected"),
        Duration::from_secs(5),
    );

    harness.assert_state("status reports rejection", |app| {
        app.status.contains("rejected")
    });
    harness.assert_screen("rejected");
}

/// Poll `harness.conn` directly (bypassing `tick()`'s drain, which only
/// exposes cumulative `App` counters, not arrival order) so the raw
/// event sequence is observable. Applies each event to `harness.app` as
/// it's polled, matching what `drain`/`tick` does, so `App` stays
/// consistent with a normal session.
fn record_order_events(harness: &mut TuiHarness, timeout: Duration) -> Vec<&'static str> {
    let mut seen = Vec::new();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while let Some(ev) = harness.conn.poll_event() {
            let tag = match &ev {
                GwEvent::Accepted { .. } => Some("Accepted"),
                GwEvent::Fill { .. } => Some("Fill"),
                GwEvent::Done { .. } => Some("Done"),
                _ => None,
            };
            harness.app.apply_event(ev);
            if let Some(tag) = tag {
                seen.push(tag);
            }
        }
        if seen.last() == Some(&"Done") {
            return seen;
        }
        thread::sleep(Duration::from_millis(5));
    }
    seen
}

/// Submit `order` on `taker` and record the raw event sequence folded
/// into `harness`, resubmitting up to `SUBMIT_ATTEMPTS` times total (no
/// `Done` seen) — see the module doc's casting-loss note.
fn submit_and_record(
    taker: &mut WsConn,
    harness: &mut TuiHarness,
    order: OrderReq,
    timeout: Duration,
) -> Vec<&'static str> {
    let mut seq = Vec::new();
    for attempt in 1..=SUBMIT_ATTEMPTS {
        taker.submit(order).expect("submit taker order");
        seq = record_order_events(harness, timeout);
        if seq.last() == Some(&"Done") {
            return seq;
        }
        eprintln!(
            "attempt {attempt}/{SUBMIT_ATTEMPTS}: no Done within {timeout:?}; \
             casting is UDP, an occasional dropped order/event is expected",
        );
    }
    seq
}

/// A resting maker gets `Accepted` (confirmed before the taker fires,
/// so the recorded sequence starts clean), then a separate taker fully
/// fills it. Asserts the fold order is exactly `Fill -> Done` after
/// that `Accepted` — invariant #1, "Fills precede ORDER_DONE".
#[test]
#[ignore = "blocked on RETURN-PATH-INTERMITTENT-DROP (bugs.md): taker fill leg \
            drops on the persistent WsConn path; run with --ignored once fixed"]
fn order_lifecycle_accepted_then_done() {
    let maker_conn = skip_if_no_cluster!(cluster::connect(5));
    let mut harness = TuiHarness::new_with(Box::new(maker_conn));
    submit_and_wait(
        &mut harness,
        OrderReq { side: Side::Buy, price: 58_000, qty: 200_000, tif: Tif::Gtc },
        |app| app.open_orders == 1,
        Duration::from_secs(5),
    );

    let mut taker = skip_if_no_cluster!(cluster::connect(99));
    let seq = submit_and_record(
        &mut taker,
        &mut harness,
        OrderReq { side: Side::Sell, price: 57_000, qty: 200_000, tif: Tif::Ioc },
        Duration::from_secs(5),
    );
    harness.tick();
    assert_eq!(seq, vec!["Fill", "Done"], "gateway ack order: {seq:?}");
    harness.assert_state("order fully filled", |app| {
        app.fills >= 1 && app.open_orders == 0
    });
    harness.assert_screen("done");
}
