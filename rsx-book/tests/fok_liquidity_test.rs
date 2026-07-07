//! Differential test: a FOK order must fail (with no fills) exactly when
//! the resting qty that crosses its limit price is less than its size,
//! and fully fill otherwise. The fast path (`can_fill_fully`, a bounded
//! price-ordered level walk over the maintained `total_qty`) is pinned to
//! an independent brute-force pass over every resting order.
//!
//! The book is driven through the real matching path with fat-tailed,
//! two-sided prices spanning several compression zones (so the sawtooth
//! and sells-resting-below-mid are exercised), then probed with random
//! FOK orders whose outcome is compared to the brute-force reference.

use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::FAIL_FOK;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo) as u64) as i64
    }
}

/// Brute-force reference: total resting qty on the opposite side that an
/// aggressor `side` at `limit` crosses — the old O(orders) semantics.
fn brute_crossing(book: &Orderbook, side: Side, limit: i64) -> i64 {
    let mut total = 0_i64;
    for lvl in book.active_levels.iter() {
        if lvl.order_count == 0 {
            continue;
        }
        let mut cur = lvl.head;
        while cur != rsx_types::NONE {
            let m = book.orders.get(cur);
            let crosses = match side {
                Side::Buy => m.side == Side::Sell as u8 && m.price.0 <= limit,
                Side::Sell => m.side == Side::Buy as u8 && m.price.0 >= limit,
            };
            if crosses {
                total += m.remaining_qty.0;
            }
            cur = m.next;
        }
    }
    total
}

fn submit_flow(book: &mut Orderbook, rng: &mut Rng, mid: i64, oid: u64) {
    let buy = rng.next() & 1 == 0;
    let off = match rng.next() % 10 {
        0..=5 => rng.range(1, 40_000),
        6..=8 => rng.range(40_000, 300_000),
        _ => rng.range(1, 5),
    };
    let price = if buy { mid - off } else { mid + off };
    let qty = rng.range(1, 500);
    let tif = if rng.next().is_multiple_of(6) {
        TimeInForce::IOC
    } else {
        TimeInForce::GTC
    };
    let mut order = IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side: if buy { Side::Buy } else { Side::Sell },
        tif,
        user_id: (oid % 64) as u32 + 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: oid,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut order);
}

/// Run one FOK probe and assert its outcome matches the brute-force
/// reference. Mutation from a successful fill is left in place (part of
/// the ongoing flow); the reference is recomputed fresh each probe.
fn probe_fok(book: &mut Orderbook, rng: &mut Rng, mid: i64, oid: u64) {
    let buy = rng.next() & 1 == 0;
    // Reach well past the touch on both sides so we hit fully-fill,
    // partial (kill), and no-cross cases.
    let off = rng.range(1, 200_000);
    let price = if buy { mid + off } else { mid - off };
    let qty = rng.range(1, 4000);
    let side = if buy { Side::Buy } else { Side::Sell };

    let avail = brute_crossing(book, side, price);
    let mut order = IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::FOK,
        user_id: (oid % 64) as u32 + 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: oid,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut order);

    let failed_fok = book.events().iter().any(|e| {
        matches!(e, Event::OrderFailed { reason, .. }
            if *reason == FAIL_FOK)
    });
    let had_fill = book
        .events()
        .iter()
        .any(|e| matches!(e, Event::Fill { .. }));

    if avail < qty {
        assert!(failed_fok, "FOK should kill: avail {avail} < qty {qty}",);
        assert!(!had_fill, "killed FOK must not fill");
    } else {
        assert!(!failed_fok, "FOK should fill: avail {avail} >= qty {qty}",);
        assert!(had_fill, "filled FOK must emit fills");
    }
}

fn count_fok_fail(book: &Orderbook) -> bool {
    book.events().iter().any(|e| {
        matches!(e, Event::OrderFailed { reason, .. }
            if *reason == FAIL_FOK)
    })
}

fn has_fill(book: &Orderbook) -> bool {
    book.events()
        .iter()
        .any(|e| matches!(e, Event::Fill { .. }))
}

/// A compressed zone (≥1) packs DISTINCT raw prices into one level.
/// `total_qty` there over-counts makers that do NOT cross the taker's
/// limit, so a FOK whose TRUE crossable qty is short must still be
/// rejected — never fall through and rest.
/// (FOK-RESTS-IN-COMPRESSED-ZONES.)
#[test]
fn fok_compressed_zone_insufficient_true_liquidity_rejected() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(), 200_000, mid);
    // Two asks in the SAME zone-1 slot (10 ticks/slot) at DISTINCT
    // prices: one crosses a buy @1_060_000, one does not.
    let crossing = book.insert_resting(1_060_000, 10, Side::Sell, 0, 1, false, 0, 1, 1);
    let non_crossing = book.insert_resting(1_060_009, 100, Side::Sell, 0, 2, false, 0, 2, 2);
    assert_eq!(
        book.orders.get(crossing).tick_index,
        book.orders.get(non_crossing).tick_index,
        "asks must share a compressed slot",
    );
    // TRUE crossable = 10; total_qty = 110. FOK for 50 must be killed.
    let mut order = IncomingOrder {
        price: 1_060_000,
        qty: 50,
        remaining_qty: 50,
        side: Side::Buy,
        tif: TimeInForce::FOK,
        user_id: 3,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 3,
    };
    process_new_order(&mut book, &mut order);

    assert!(count_fok_fail(&book), "FOK must be rejected");
    assert!(!has_fill(&book), "rejected FOK must leave zero fills");
    // Book untouched: both makers still rest.
    assert!(book.orders.get(crossing).is_active());
    assert!(book.orders.get(non_crossing).is_active());
    assert_ne!(book.best_ask_tick, rsx_types::NONE);
}

/// Same hazard with tick_size != 1 (a slot spans compression*tick_size
/// raw price, so PENGU-style tick=1 is not the only shape). FOK short on
/// true liquidity in a far compressed zone is rejected.
#[test]
fn fok_compressed_zone_tick50_insufficient_rejected() {
    let cfg = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 50,
        lot_size: 1,
    };
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(cfg, 200_000, mid);
    // zone-1 slot spans 10*50 = 500 raw price; both asks land in it.
    let crossing = book.insert_resting(1_060_000, 10, Side::Sell, 0, 1, false, 0, 1, 1);
    let non_crossing = book.insert_resting(1_060_450, 100, Side::Sell, 0, 2, false, 0, 2, 2);
    assert_eq!(
        book.orders.get(crossing).tick_index,
        book.orders.get(non_crossing).tick_index,
        "asks must share a compressed slot",
    );
    let mut order = IncomingOrder {
        price: 1_060_000,
        qty: 50,
        remaining_qty: 50,
        side: Side::Buy,
        tif: TimeInForce::FOK,
        user_id: 3,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 3,
    };
    process_new_order(&mut book, &mut order);

    assert!(count_fok_fail(&book), "FOK must be rejected");
    assert!(!has_fill(&book), "rejected FOK must leave zero fills");
    assert!(book.orders.get(crossing).is_active());
    assert!(book.orders.get(non_crossing).is_active());
}

/// Positive control: a FOK in a compressed zone whose TRUE crossable qty
/// IS sufficient still fully fills (the accurate walk must not
/// under-count either).
#[test]
fn fok_compressed_zone_sufficient_liquidity_fills() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(), 200_000, mid);
    // Two asks in one zone-1 slot, BOTH crossing a buy @1_060_009.
    book.insert_resting(1_060_000, 30, Side::Sell, 0, 1, false, 0, 1, 1);
    book.insert_resting(1_060_009, 40, Side::Sell, 0, 2, false, 0, 2, 2);
    let mut order = IncomingOrder {
        price: 1_060_009,
        qty: 50,
        remaining_qty: 50,
        side: Side::Buy,
        tif: TimeInForce::FOK,
        user_id: 3,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 3,
    };
    process_new_order(&mut book, &mut order);

    assert!(!count_fok_fail(&book), "sufficient FOK must not fail");
    assert!(has_fill(&book), "sufficient FOK must fill");
    let done_filled = book.events().iter().any(|e| {
        matches!(e, Event::OrderDone { filled_qty, remaining_qty, .. }
            if filled_qty.0 == 50 && remaining_qty.0 == 0)
    });
    assert!(done_filled, "FOK must fully fill 50");
}

#[test]
fn fok_outcome_matches_bruteforce_under_random_flow() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(), 200_000, mid);
    let mut rng = Rng(0xF0F0_1234_5678_9ABC);
    let mut oid = 1_u64;

    // Build depth first so probes have liquidity to reason about.
    for _ in 0..2000 {
        submit_flow(&mut book, &mut rng, mid, oid);
        oid += 1;
    }

    for _ in 0..3000 {
        if rng.next().is_multiple_of(4) {
            probe_fok(&mut book, &mut rng, mid, oid);
        } else {
            submit_flow(&mut book, &mut rng, mid, oid);
        }
        oid += 1;
    }
}
