//! Live-cluster dialer for the ported e2e suites (T3-T5): connect a
//! real `WsConn` to whatever gateway `RSX_GW_LISTEN` names, seed a
//! resting maker order (mirroring `scripts/demo-trade.sh`), and a
//! `skip_if_no_cluster!` helper so these tests are no-ops when nothing
//! is listening (the `rsx-playground/tests/live/conftest.py`
//! skip-if-down pattern, ported to Rust).
//!
//! Unused until T3/T4/T5 add the tests that call `connect`/`seed_book`
//! — `support_smoke.rs` (T2's own acceptance test) only exercises
//! `harness.rs` over a `MockConn`, so clippy would otherwise flag this
//! whole module dead.
#![allow(dead_code)]

use rsx_tui::conn::GatewayConn;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::OrderReq;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::ws::gateway_url;
use rsx_tui::ws::jwt_secret;
use rsx_tui::ws::mint_jwt;
use rsx_tui::ws::WsConn;
use std::time::Duration;
use std::time::Instant;

/// Symbol id every harness test trades, matching `scripts/demo-trade.sh`'s
/// `SYMBOL_ID=10` so a `seed_book` maker and a harness taker cross on
/// the same instrument as the shell demo.
pub const SYMBOL_ID: u32 = 10;

/// How long to wait for the handshake to confirm (`Connected`) before
/// declaring the cluster unreachable.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Dial the gateway for `user_id` and wait for the connection to
/// confirm. Returns `None` (never panics) if nothing is listening at
/// `RSX_GW_LISTEN` or the handshake fails/times out — callers pair
/// this with `skip_if_no_cluster!` to skip cleanly with no cluster up.
pub fn connect(user_id: u32) -> Option<WsConn> {
    let token = mint_jwt(user_id, &jwt_secret());
    let mut conn = WsConn::connect(gateway_url(), token, SYMBOL_ID).ok()?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    while Instant::now() < deadline {
        match conn.poll_event() {
            Some(GwEvent::Connected) => return Some(conn),
            Some(GwEvent::Disconnected) => return None,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Post a resting maker BUY so a subsequent crossing SELL has a bid to
/// fill against. Price 49_000 sits below the demo/bench book's ~50_000
/// resting asks so it RESTS (a buy at/above the best ask crosses it and
/// never rests — the old `submit_ioc_fills` bug), and above the demo
/// bids so it becomes the best bid. Qty matches the `submit_ioc_fills`
/// taker (500_000) so the taker fully consumes it, leaving no resting
/// bid to pollute the shared long-lived book for the next test.
pub fn seed_book(conn: &mut WsConn) {
    conn.submit(OrderReq {
        side: Side::Buy,
        price: 49_000,
        qty: 500_000,
        tif: Tif::Gtc,
    })
    .expect("seed_book: submit maker order");
}

/// `eprintln!` why a test is skipping and return early. Use at the top
/// of every cluster-gated test: `let mut conn =
/// support::cluster::skip_if_no_cluster!(support::cluster::connect(1));`
#[macro_export]
macro_rules! skip_if_no_cluster {
    ($maybe_conn:expr) => {
        match $maybe_conn {
            Some(conn) => conn,
            None => {
                eprintln!(
                    "skip: no cluster reachable at RSX_GW_LISTEN, \
                     skipping test (start one with \
                     `./rsx-playground/playground start-all minimal`)",
                );
                return;
            }
        }
    };
}
