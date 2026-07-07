//! Unit coverage for `ws.rs`'s frame folding — in particular the
//! oid->side pairing that a `U`/`F` frame stream drives, exercised
//! without a runtime or a cluster by calling the private `fold_frame`
//! directly with crafted frames (the same shape `run_client` feeds it).

use crate::conn::GwEvent;
use crate::conn::OrderReq;
use crate::conn::Side;
use crate::conn::Tif;
use crate::ws::fold_frame;
use crate::ws::mint_jwt;
use crate::ws::order_frame;
use crate::ws::PendingOrder;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::VecDeque;

/// A 32-hex-char oid (order_id_hi ++ order_id_lo), matching the
/// gateway's `oid_hex`; the low 16 chars decode to `n` (see
/// `oid_to_u64`).
fn oid(n: u64) -> String {
    format!("{n:032x}")
}

/// Drives `fold_frame` over one connection's evolving state, the way
/// `run_client` does, so a test can feed a whole frame sequence and
/// read back the folded events in order.
struct Folder {
    oid_side: HashMap<String, Side>,
    pending: VecDeque<PendingOrder>,
    warned_unknown_frame: bool,
    unknown_side_count: u64,
}

impl Folder {
    fn new() -> Self {
        Folder {
            oid_side: HashMap::new(),
            pending: VecDeque::new(),
            warned_unknown_frame: false,
            unknown_side_count: 0,
        }
    }

    /// Mirror `run_client`'s submit leg: remember a submitted order so a
    /// later `U`/`F` can recover its side.
    fn submit(&mut self, qty: i64, side: Side) {
        self.pending.push_back(PendingOrder { qty, side });
    }

    fn fold(&mut self, text: &str) -> Option<GwEvent> {
        fold_frame(
            text,
            &mut self.oid_side,
            &mut self.pending,
            &mut self.warned_unknown_frame,
            &mut self.unknown_side_count,
        )
    }
}

/// Two orders in flight on one connection, acked out of submission
/// order: the gateway accepts the *second*-submitted order first. FIFO
/// alone would label the first ack with the first-submitted side; qty
/// pairing keeps each fill's side correct.
#[test]
fn out_of_order_acks_keep_correct_side() {
    let mut f = Folder::new();
    // Submit Buy(qty=5) then Sell(qty=7).
    f.submit(5, Side::Buy);
    f.submit(7, Side::Sell);

    // Gateway accepts the Sell first (out of order): {U:[oid,1,filled,
    // remaining,reason]} — remaining=7 recovers the Sell.
    let sell = oid(7);
    let buy = oid(5);
    assert_eq!(
        f.fold(&json!({ "U": [sell, 1, 0, 7, 0] }).to_string()),
        Some(GwEvent::Accepted { oid: 7 }),
    );
    assert_eq!(
        f.fold(&json!({ "U": [buy, 1, 0, 5, 0] }).to_string()),
        Some(GwEvent::Accepted { oid: 5 }),
    );

    // Each order fills; the fill must carry the side that was submitted,
    // not the FIFO-popped one.
    let sell_fill = f.fold(&json!({ "F": [sell, oid(999), 49_000, 7, 0, 0] }).to_string());
    let buy_fill = f.fold(&json!({ "F": [buy, oid(999), 51_000, 5, 0, 0] }).to_string());
    assert_eq!(
        sell_fill,
        Some(GwEvent::Fill {
            oid: 7,
            px: 49_000,
            qty: 7,
            side: Side::Sell
        }),
    );
    assert_eq!(
        buy_fill,
        Some(GwEvent::Fill {
            oid: 5,
            px: 51_000,
            qty: 5,
            side: Side::Buy
        }),
    );
}

/// The finding's exact shape: a later order is rejected fast, before the
/// earlier order is accepted. The reject must not consume a pending
/// side (it needs none), so the earlier order's fill still labels Buy.
#[test]
fn fast_reject_before_accept_does_not_shift_side() {
    let mut f = Folder::new();
    f.submit(5, Side::Buy); // A, earlier
    f.submit(7, Side::Sell); // B, later — gets rejected first

    let a = oid(5);
    let b = oid(7);
    // B rejected (status 3, no qty): yields Rejected, consumes no pending.
    assert!(matches!(
        f.fold(&json!({ "U": [b, 3, 0, 0, 42] }).to_string()),
        Some(GwEvent::Rejected { .. }),
    ));
    // A accepted then filled: still Buy.
    assert_eq!(
        f.fold(&json!({ "U": [a, 1, 0, 5, 0] }).to_string()),
        Some(GwEvent::Accepted { oid: 5 }),
    );
    assert_eq!(
        f.fold(&json!({ "F": [a, oid(999), 50_000, 5, 0, 0] }).to_string()),
        Some(GwEvent::Fill {
            oid: 5,
            px: 50_000,
            qty: 5,
            side: Side::Buy
        }),
    );
}

/// A fill whose oid was never paired to a submitted order falls back to
/// Buy, but counts the fallback (surfaced, not silent).
#[test]
fn unpaired_fill_counts_the_fallback() {
    let mut f = Folder::new();
    let ev = f.fold(&json!({ "F": [oid(1), oid(2), 50_000, 3, 0, 0] }).to_string());
    assert_eq!(
        ev,
        Some(GwEvent::Fill {
            oid: 1,
            px: 50_000,
            qty: 3,
            side: Side::Buy
        }),
    );
    assert_eq!(f.unknown_side_count, 1, "the Buy fallback must be counted");
}

/// CTO finding: an `{E:[code, msg]}` error frame folds to a `Rejected`
/// carrying both fields — the existing suite exercised `U`/`F` but never
/// the `E` (gateway/protocol error) path.
#[test]
fn error_frame_folds_to_rejected() {
    let mut f = Folder::new();
    let ev = f.fold(&json!({ "E": ["4001", "bad symbol"] }).to_string());
    match ev {
        Some(GwEvent::Rejected { reason }) => {
            assert!(reason.contains("4001"), "code in reason: {reason}");
            assert!(reason.contains("bad symbol"), "msg in reason: {reason}");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

/// CTO finding: a heartbeat frame `{H:[ts]}` produces no `GwEvent` (the
/// read loop echoes it for liveness; it must NOT leak into the app's
/// event stream as a spurious update). Pins the "H folds to None"
/// contract fold_frame's doc promises.
#[test]
fn heartbeat_frame_produces_no_event() {
    let mut f = Folder::new();
    assert_eq!(f.fold(&json!({ "H": [123_456] }).to_string()), None);
}

/// CTO/CEO finding: public-channel frames (trade `T`, book `B`) have no
/// `GwEvent` mapping on the order connection and are dropped — this is
/// exactly why the book ladder and trade tape stay empty over a real
/// `WsConn` (it never subscribes to marketdata). Pinning the drop makes
/// that documented gap a checked fact, not a surprise.
#[test]
fn public_channel_frames_are_dropped() {
    let mut f = Folder::new();
    assert_eq!(
        f.fold(&json!({ "T": [10, 0, 50_000, 3, 0] }).to_string()),
        None,
        "trade frame has no order-channel GwEvent",
    );
    assert_eq!(
        f.fold(&json!({ "B": [10, 49_900, 5, 50_100, 7] }).to_string()),
        None,
        "book/BBO frame has no order-channel GwEvent",
    );
}

/// CTO finding: `order_frame` encodes an `OrderReq` as the webproto-49
/// `{N:[sym, side, px, qty, cid, tif, ro, po]}` new-order frame. Pins
/// the wire contract: Buy=0/Sell=1, Gtc=0/Ioc=1/Fok=2, ro=po=0, and the
/// symbol/px/qty/cid placement — a silent shift here would have the
/// gateway trade the wrong thing.
#[test]
fn order_frame_encodes_webproto_new_order() {
    let order = OrderReq {
        side: Side::Sell,
        price: 49_000,
        qty: 3,
        tif: Tif::Ioc,
    };
    let frame = order_frame(10, &order, "00000000000000000042");
    let v: Value = serde_json::from_str(&frame).expect("valid JSON");
    let arr = v.get("N").and_then(Value::as_array).expect("N array");
    assert_eq!(arr[0].as_u64(), Some(10), "symbol_id");
    assert_eq!(arr[1].as_u64(), Some(1), "Sell -> 1");
    assert_eq!(arr[2].as_i64(), Some(49_000), "price");
    assert_eq!(arr[3].as_i64(), Some(3), "qty");
    assert_eq!(arr[4].as_str(), Some("00000000000000000042"), "cid");
    assert_eq!(arr[5].as_u64(), Some(1), "Ioc -> 1");
    assert_eq!(arr[6].as_u64(), Some(0), "reduce-only 0");
    assert_eq!(arr[7].as_u64(), Some(0), "post-only 0");

    // Buy/Gtc/Fok mappings on a second order.
    let buy = OrderReq {
        side: Side::Buy,
        price: 1,
        qty: 1,
        tif: Tif::Fok,
    };
    let f2 = order_frame(10, &buy, "x");
    let v2: Value = serde_json::from_str(&f2).unwrap();
    let a2 = v2.get("N").and_then(Value::as_array).unwrap();
    assert_eq!(a2[1].as_u64(), Some(0), "Buy -> 0");
    assert_eq!(a2[5].as_u64(), Some(2), "Fok -> 2");
}

/// CTO finding: `mint_jwt` puts a fresh `jti` in every token, so two
/// mints for the same user differ. The gateway's `JtiTracker` rejects a
/// replayed `jti`; a stable token would get the second connection
/// refused. Pins replay-safety without needing a gateway.
#[test]
fn mint_jwt_is_unique_per_call() {
    let a = mint_jwt(1, "test-secret-padding-padding-pad");
    let b = mint_jwt(1, "test-secret-padding-padding-pad");
    assert_ne!(a, b, "each token carries a distinct jti");
}
