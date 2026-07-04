//! Submit/skip helpers shared by the cluster-gated e2e suites, factored
//! out of the byte-identical copies `e2e_orders`/`e2e_positions`/
//! `e2e_book`/`e2e_guarantees` each grew.
//!
//! Each `tests/*.rs` is its own crate and pulls in only the pieces it
//! uses, so some helpers read as dead per binary — mirror `cluster.rs`.
#![allow(dead_code)]

use super::harness::TuiHarness;
use rsx_tui::App;
use rsx_tui::conn::OrderReq;
use std::time::Duration;

/// Up to this many submit attempts before giving up on an ack. Casting
/// is UDP: an occasional dropped order/event is expected by design (see
/// `e2e_orders.rs`'s module doc). A fresh cid each retry, so a resend is
/// a new attempt, not a WAL-deduped replay.
pub const SUBMIT_ATTEMPTS: u32 = 2;

/// Submit `order` over `harness.conn` and wait for `pred`. Retries (a
/// fresh cid) up to `SUBMIT_ATTEMPTS` on the UDP-drop path, then panics
/// if never acked.
///
/// Finding 2 (return-path masking): a fill that arrives only *after* a
/// resubmit FAILS the test rather than passing silently. A silently
/// accepted resubmit is exactly how a residual downstream drop (ME
/// emits, risk never sees) would hide behind the retry. The retry is
/// kept — it turns a total loss into a clear "never acked" and a
/// transient into a loud "needed a resubmit", instead of a green pass.
pub fn submit_and_wait<F>(
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
            assert_eq!(
                attempt, 1,
                "return-path masking (finding 2): predicate met only on \
                 attempt {attempt}; a resubmit papering over a dropped \
                 first-attempt ack/fill is the ME-emits-but-risk-never-sees \
                 signature — investigate, do not mask",
            );
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

/// `wait_for`, but a timeout is an explicit named skip (not a failure)
/// for a signal known-unwired on `WsConn` today (the `B`/`T`/`P`
/// marketdata channels — see `e2e_book.rs`'s module doc). Returns
/// whether the signal arrived.
pub fn wait_or_skip_gap(
    harness: &mut TuiHarness,
    pred: impl Fn(&App) -> bool,
    timeout: Duration,
    gap: &str,
) -> bool {
    if harness.wait_for(pred, timeout).is_some() {
        true
    } else {
        eprintln!("skip: {gap}");
        false
    }
}
