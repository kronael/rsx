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
                Side::Buy => {
                    m.side == Side::Sell as u8
                        && m.price.0 <= limit
                }
                Side::Sell => {
                    m.side == Side::Buy as u8
                        && m.price.0 >= limit
                }
            };
            if crosses {
                total += m.remaining_qty.0;
            }
            cur = m.next;
        }
    }
    total
}

fn submit_flow(
    book: &mut Orderbook,
    rng: &mut Rng,
    mid: i64,
    oid: u64,
) {
    let buy = rng.next() & 1 == 0;
    let off = match rng.next() % 10 {
        0..=5 => rng.range(1, 40_000),
        6..=8 => rng.range(40_000, 300_000),
        _ => rng.range(1, 5),
    };
    let price = if buy { mid - off } else { mid + off };
    let qty = rng.range(1, 500);
    let tif = if rng.next() % 6 == 0 {
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
fn probe_fok(
    book: &mut Orderbook,
    rng: &mut Rng,
    mid: i64,
    oid: u64,
) {
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
        assert!(
            failed_fok,
            "FOK should kill: avail {avail} < qty {qty}",
        );
        assert!(!had_fill, "killed FOK must not fill");
    } else {
        assert!(
            !failed_fok,
            "FOK should fill: avail {avail} >= qty {qty}",
        );
        assert!(had_fill, "filled FOK must emit fills");
    }
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
        if rng.next() % 4 == 0 {
            probe_fok(&mut book, &mut rng, mid, oid);
        } else {
            submit_flow(&mut book, &mut rng, mid, oid);
        }
        oid += 1;
    }
}
