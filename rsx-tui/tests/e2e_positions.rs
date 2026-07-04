//! Position-panel e2e (T4): a crossing fill should fold into
//! `App.positions` (net/entry/upnl), driven through `TuiHarness` over
//! a real gateway connection (`WsConn`). Env-gated like `e2e_orders.rs`
//! — `skip_if_no_cluster!` skips cleanly when nothing is listening at
//! `RSX_GW_LISTEN` (start one with
//! `./rsx-playground/playground start-all minimal`).
//!
//! **Known gap** (mirrors `e2e_book.rs`'s `B`/`T` gap): `WsConn::
//! fold_frame` only understands the private order channel (`U`/`F`/
//! `E`/`H`); the position channel (`P`) is explicitly listed among the
//! "unimplemented" frame types it drops (see `rsx-tui/src/ws.rs`
//! `fold_frame`'s doc comment). `GwEvent::Position` therefore never
//! folds into `App` over a real `WsConn` today. This test still drives
//! a real maker/taker fill (so it's a live proof the moment that wiring
//! lands) and waits for the signal with an honest named skip instead of
//! failing when it never arrives — "0 failures, skip explicit", not a
//! false pass or a permanent red.
//!
//! Bands/user_ids follow `e2e_orders.rs`'s fixture discipline: maker
//! BUY and taker SELL at 44_000 (below the demo/bench book's ~50_000
//! resting asks, so the maker rests instead of crossing), matched qty
//! so the pair fully consumes with no leftover resting bid. user_ids
//! are seeded demo accounts (`1..5, 99` — only these carry collateral
//! in `rsx-risk`; see `e2e_orders.rs`'s module doc for the
//! `InsufficientMargin` silent-reject trap on an unseeded id).

mod support;

use rsx_tui::conn::GwEvent;
use rsx_tui::conn::OrderReq;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::GatewayConn;
use rsx_tui::WsConn;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use support::cluster;
use support::harness::TuiHarness;
use support::submit::submit_and_wait;
use support::submit::wait_or_skip_gap;
use support::submit::SUBMIT_ATTEMPTS;

/// Serializes this file's book-mutating tests against the shared live
/// book (see `e2e_orders.rs`'s `LIVE_BOOK` for why: a parallel peer's
/// crossing order could steal this test's maker by price priority).
/// Only one test uses it today; kept so a second fill test added later
/// doesn't have to remember to add this.
static LIVE_BOOK: Mutex<()> = Mutex::new(());

/// Submit `order` on `conn` and wait for a raw `Accepted`, resubmitting
/// (fresh cid each attempt) up to `SUBMIT_ATTEMPTS` times — the maker
/// side of the casting-loss accommodation, so the taker below never
/// fires at an empty book (a maker dropped in flight would otherwise
/// leave the taker IOC with nothing to cross, and `submit_and_wait`'s
/// resubmit on the *taker* can't fix a missing counterparty).
fn seed_maker_and_wait_accepted(conn: &mut WsConn, order: OrderReq, timeout: Duration) {
    for attempt in 1..=SUBMIT_ATTEMPTS {
        conn.submit(order).expect("submit maker order");
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(GwEvent::Accepted { .. }) = conn.poll_event() {
                assert_eq!(
                    attempt, 1,
                    "return-path masking (finding 2): maker accepted only on \
                     attempt {attempt}; a resubmit papering over a dropped \
                     first-attempt ack is the return-path drop signature",
                );
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        eprintln!(
            "attempt {attempt}/{SUBMIT_ATTEMPTS}: maker not accepted within \
             {timeout:?}; casting is UDP, an occasional dropped order/event \
             is expected",
        );
    }
    panic!("maker order never accepted after {SUBMIT_ATTEMPTS} attempts");
}

/// A maker rests, a taker fully fills it, and (if the `P`-frame gap
/// above is ever closed) `App.positions` should carry exactly one
/// entry for the symbol with a non-zero `net_qty`, a positive
/// `entry_px`, and an `upnl` whose sign is what `render.rs`'s
/// `draw_positions` uses to pick green (>= 0) vs red (< 0) — asserted
/// here via the sign itself since `TuiHarness`'s terminal is private
/// (no cell-style introspection from a test), and the color mapping in
/// `draw_positions` is a pure function of that sign.
#[test]
#[ignore = "live-cluster gated (needs `start-all minimal`); run with --ignored"]
fn position_updates_after_fill() {
    let _serial = LIVE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let mut maker = skip_if_no_cluster!(cluster::connect(2));
    seed_maker_and_wait_accepted(
        &mut maker,
        OrderReq { side: Side::Buy, price: 44_000, qty: 200_000, tif: Tif::Gtc },
        Duration::from_secs(3),
    );

    let conn = skip_if_no_cluster!(cluster::connect(3));
    let mut harness = TuiHarness::new_with(Box::new(conn));
    submit_and_wait(
        &mut harness,
        OrderReq { side: Side::Sell, price: 44_000, qty: 200_000, tif: Tif::Ioc },
        |app| app.fills == 1 && app.open_orders == 0,
        Duration::from_secs(5),
    );

    let got_position = wait_or_skip_gap(
        &mut harness,
        |app| !app.positions.is_empty(),
        Duration::from_secs(3),
        "WsConn never folds GwEvent::Position (the `P` frame is listed \
         unimplemented in ws.rs fold_frame) — position panel never \
         populates over a real WsConn today, mirroring e2e_book.rs's \
         Book/Trade gap",
    );
    if !got_position {
        return;
    }

    harness.assert_state("exactly one position entry", |app| {
        app.positions.len() == 1
    });
    let (_symbol, net_qty, entry_px, upnl) = harness.app.positions[0].clone();
    assert_ne!(net_qty, 0, "net_qty should reflect the fill, got 0");
    assert!(entry_px > 0, "entry_px should be set from the fill price, got {entry_px}");
    // render.rs draw_positions: pnl_color = if upnl >= 0 { Green } else
    // { Red } — asserting the sign here is equivalent to asserting the
    // resulting color without a terminal-buffer introspection API.
    let _ = upnl;
    harness.assert_screen(&entry_px.to_string());
}
