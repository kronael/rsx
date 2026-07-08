//! Regression tests for the compressed / mixed-slot correctness bugs
//! (BUGS.md "Status — 2026-07-07"). A compression slot outside zone 0 can
//! hold BOTH order sides and multiple raw prices; each of these encodes a
//! repro that asserts CORRECT behaviour. They fail against the pre-fix
//! matching engine (same-side fill / ME panic / FOK partial / wrongful
//! post-only cancel) and pass after.

use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::CANCEL_POST_ONLY;
use rsx_book::event::FAIL_FOK;
use rsx_book::event::REASON_FILLED;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

fn config(tick_size: i64) -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size,
        lot_size: 1,
    }
}

fn order(
    price: i64,
    qty: i64,
    side: Side,
    tif: TimeInForce,
    user_id: u32,
    post_only: bool,
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif,
        user_id,
        reduce_only: false,
        post_only,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: user_id as u64,
    }
}

fn gtc(price: i64, qty: i64, side: Side, user_id: u32) -> IncomingOrder {
    order(price, qty, side, TimeInForce::GTC, user_id, false)
}

fn net_qty(book: &Orderbook, user_id: u32) -> i64 {
    book.users.net_qty(user_id).unwrap_or(0)
}

fn fills(book: &Orderbook) -> Vec<(i64, i64, u32)> {
    book.events()
        .iter()
        .filter_map(|e| match e {
            Event::Fill {
                price,
                qty,
                maker_user_id,
                ..
            } => Some((price.0, qty.0, *maker_user_id)),
            _ => None,
        })
        .collect()
}

/// A best bid < best ask must hold whenever both sides rest.
fn assert_uncrossed(book: &Orderbook) {
    if book.best_bid_px != 0 && book.best_ask_px != 0 {
        assert!(
            book.best_bid_px < book.best_ask_px,
            "crossed book: bid {} >= ask {}",
            book.best_bid_px,
            book.best_ask_px,
        );
    }
}

/// BOOK-MIXED-SIDE-SELF-TRADE: a sell taker must fill the resting BUY in a
/// mixed slot, NEVER the resting SELL sharing the same compressed tick.
#[test]
fn sell_taker_does_not_self_trade_resting_sell() {
    // mid=50_000 tick=1: 47_491/47_495/47_490 all land in one zone-1 slot.
    let mut book = Orderbook::new(config(1), 1024, 50_000);
    process_new_order(&mut book, &mut gtc(47_491, 10, Side::Buy, 1));
    process_new_order(&mut book, &mut gtc(47_495, 10, Side::Sell, 2));
    // Taker sell crosses the resting buy (47_491) only.
    process_new_order(&mut book, &mut gtc(47_490, 20, Side::Sell, 3));

    let f = fills(&book);
    assert_eq!(f.len(), 1, "exactly one fill; got {f:?}");
    assert_eq!(f[0], (47_491, 10, 1), "must fill the resting BUY (user 1)");
    // The resting SELL (user 2) was never touched: its net stays flat and
    // no fill priced at 47_495 was produced.
    assert_eq!(net_qty(&book, 2), 0, "resting sell wrongly self-traded");
    assert!(
        !book
            .events()
            .iter()
            .any(|e| matches!(e, Event::Fill { price, .. } if price.0 == 47_495)),
        "produced a same-side fill against the resting sell",
    );
    // Taker sold 10 (residual 10 rests); seller-3 net = -10.
    assert_eq!(net_qty(&book, 3), -10);
    assert_eq!(net_qty(&book, 1), 10);
    assert!(book
        .events()
        .iter()
        .any(|e| matches!(e, Event::OrderInserted { price, .. } if price.0 == 47_490)));
    assert_uncrossed(&book);
}

/// BOOK-STALE-OCC-ME-CRASH: the full repro sequence must not panic (stale
/// occupancy pointed a best tick at an empty level -> emit_bbo head==NONE
/// deref) and must leave the book uncrossed.
#[test]
fn stale_occupancy_sequence_does_not_panic() {
    let mut book = Orderbook::new(config(1), 1024, 50_000);
    process_new_order(&mut book, &mut gtc(47_491, 10, Side::Buy, 1));
    process_new_order(&mut book, &mut gtc(47_495, 10, Side::Sell, 2));
    process_new_order(&mut book, &mut gtc(47_490, 20, Side::Sell, 3));
    assert_uncrossed(&book);
    // These two additionally-crash the pre-fix engine via the stale bit.
    process_new_order(&mut book, &mut gtc(47_000, 10, Side::Buy, 4));
    assert_uncrossed(&book);
    process_new_order(&mut book, &mut gtc(46_900, 10, Side::Sell, 5));
    assert_uncrossed(&book);
    // One more crossing taker to force a best-tick rescan + BBO emit.
    process_new_order(&mut book, &mut gtc(47_495, 40, Side::Buy, 6));
    assert_uncrossed(&book);
}

/// BOOK-FOK-CLAMP-DIVERGENCE: FOK feasibility must equal the real fill.
/// With only 10 crossing, a FOK for 20 must FAIL (no fill, no rest), not
/// partially fill and then panic / double-complete.
#[test]
fn fok_feasibility_equals_fill_insufficient_rejected() {
    // mid=50_001 tick=3: 52_497 (dist 2496) and 52_500 (dist 2499) are
    // both in zone 0 and must occupy DISTINCT slots after the z0 fix.
    let mut book = Orderbook::new(config(3), 1024, 50_001);
    process_new_order(&mut book, &mut gtc(52_497, 10, Side::Sell, 1));
    process_new_order(&mut book, &mut gtc(52_500, 10, Side::Sell, 2));

    // Only 52_497 crosses a buy limit of 52_497 -> 10 < 20 -> reject.
    let mut fok = order(52_497, 20, Side::Buy, TimeInForce::FOK, 3, false);
    process_new_order(&mut book, &mut fok);

    assert!(
        book.events()
            .iter()
            .any(|e| matches!(e, Event::OrderFailed { reason, .. } if *reason == FAIL_FOK)),
        "insufficient FOK must fail; events={:?}",
        book.events(),
    );
    assert_eq!(fills(&book).len(), 0, "FOK that fails must emit no fills");
    assert!(
        !book
            .events()
            .iter()
            .any(|e| matches!(e, Event::OrderInserted { .. })),
        "a failed FOK must never rest",
    );
    // Both makers untouched.
    assert_eq!(net_qty(&book, 1), 0);
    assert_eq!(net_qty(&book, 2), 0);
}

/// The other half: when liquidity truly suffices, the FOK fully fills.
#[test]
fn fok_feasibility_equals_fill_sufficient_fills() {
    let mut book = Orderbook::new(config(3), 1024, 50_001);
    process_new_order(&mut book, &mut gtc(52_497, 10, Side::Sell, 1));
    process_new_order(&mut book, &mut gtc(52_500, 10, Side::Sell, 2));

    // Buy limit 52_500 crosses both sells (10 + 10 = 20) -> fully fills.
    let mut fok = order(52_500, 20, Side::Buy, TimeInForce::FOK, 3, false);
    process_new_order(&mut book, &mut fok);

    let f = fills(&book);
    assert_eq!(f.len(), 2, "both makers fill; got {f:?}");
    assert_eq!(f.iter().map(|x| x.1).sum::<i64>(), 20);
    assert!(
        book.events()
            .iter()
            .any(|e| matches!(e, Event::OrderDone { reason, .. } if *reason == REASON_FILLED)),
        "sufficient FOK must complete FILLED",
    );
    assert!(!book
        .events()
        .iter()
        .any(|e| matches!(e, Event::OrderFailed { .. })));
    assert_eq!(net_qty(&book, 3), 20);
}

/// BOOK-STALE-BBA-WRONGFUL-POSTONLY: cancelling the best-priced order in a
/// mixed/compressed slot must refresh best_ask_px even though the slot is
/// only partially emptied, so a non-crossing post-only order rests.
#[test]
fn post_only_not_cancelled_after_partial_slot_empty() {
    // mid=50_000 tick=1: 52_501 and 52_509 share one zone-1 slot.
    let mut book = Orderbook::new(config(1), 1024, 50_000);
    let h1 = book.insert_resting(52_501, 100, Side::Sell, 0, 1, false, 0, 0, 1);
    book.insert_resting(52_509, 100, Side::Sell, 0, 2, false, 0, 0, 2);
    assert_eq!(book.best_ask_px, 52_501);

    // Cancel the best (52_501); best ask must fall back to 52_509, not stay
    // stale at 52_501 (the pre-fix bug rescanned only on full-empty).
    assert!(book.cancel_order(h1));
    assert_eq!(book.best_ask_px, 52_509, "stale best_ask_px after cancel");

    // Post-only BUY 52_505 crosses nothing (best ask is 52_509) -> rests.
    let mut po = order(52_505, 100, Side::Buy, TimeInForce::GTC, 3, true);
    process_new_order(&mut book, &mut po);
    assert!(
        book.events()
            .iter()
            .any(|e| matches!(e, Event::OrderInserted { .. })),
        "non-crossing post-only wrongly rejected; events={:?}",
        book.events(),
    );
    assert!(
        !book.events().iter().any(
            |e| matches!(e, Event::OrderCancelled { reason, .. } if *reason == CANCEL_POST_ONLY)
        ),
        "post-only wrongly CANCEL_POST_ONLY",
    );
}
