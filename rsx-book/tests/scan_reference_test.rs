//! Differential test: the bitmap-driven `scan_next_bid`/`scan_next_ask`
//! must agree with an independent brute-force pass over `active_levels`
//! for every reachable book state. The brute-force reference is the old
//! O(slots) scan (max BUY head price / min SELL head price), so this
//! pins the new fast path to identical semantics.
//!
//! The book is driven through the real matching path
//! (`process_new_order` then `cancel_order`) with fat-tailed prices that
//! span multiple compression zones, plus a recenter, so the sawtooth and
//! the deep-level clears are exercised — not just the near-BBO case.

use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rsx_types::NONE;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

/// Brute-force reference: tick of the highest-priced BUY-head level.
fn ref_bid(book: &Orderbook) -> u32 {
    let mut best = NONE;
    let mut best_px = i64::MIN;
    for (i, lvl) in book.active_levels.iter().enumerate() {
        if lvl.order_count == 0 {
            continue;
        }
        let head = book.orders.get(lvl.head);
        if head.side != Side::Buy as u8 {
            continue;
        }
        if best == NONE || head.price.0 > best_px {
            best = i as u32;
            best_px = head.price.0;
        }
    }
    best
}

/// Brute-force reference: tick of the lowest-priced SELL-head level.
fn ref_ask(book: &Orderbook) -> u32 {
    let mut best = NONE;
    let mut best_px = i64::MAX;
    for (i, lvl) in book.active_levels.iter().enumerate() {
        if lvl.order_count == 0 {
            continue;
        }
        let head = book.orders.get(lvl.head);
        if head.side != Side::Sell as u8 {
            continue;
        }
        if best == NONE || head.price.0 < best_px {
            best = i as u32;
            best_px = head.price.0;
        }
    }
    best
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

/// After every op the fast scan must equal the brute-force reference,
/// the tracked BBA ticks must equal the scans, and the book must stay
/// uncrossed.
fn check(book: &Orderbook) {
    let rb = ref_bid(book);
    let ra = ref_ask(book);
    assert_eq!(book.scan_next_bid(NONE), rb, "bid scan diverged");
    assert_eq!(book.scan_next_ask(NONE), ra, "ask scan diverged");
    assert_eq!(book.best_bid_tick, rb, "best_bid_tick stale");
    assert_eq!(book.best_ask_tick, ra, "best_ask_tick stale");
    if rb != NONE && ra != NONE {
        assert!(
            book.best_bid_px < book.best_ask_px,
            "crossed book: bid {} >= ask {}",
            book.best_bid_px,
            book.best_ask_px,
        );
    }
}

fn submit(
    book: &mut Orderbook,
    live: &mut Vec<u32>,
    rng: &mut Rng,
    mid: i64,
    oid: u64,
) {
    let buy = rng.next() & 1 == 0;
    // Fat-ish offset spanning several zones (mid=1_000_000, tick 1:
    // zone 0 ~<50k, out to ~450k). Occasionally price through the
    // spread to force matches / level clears.
    let off = match rng.next() % 10 {
        0..=5 => rng.range(1, 40_000),
        6..=8 => rng.range(40_000, 300_000),
        _ => rng.range(1, 5),
    };
    let price = if buy { mid - off } else { mid + off };
    let qty = rng.range(1, 500);
    let tif = match rng.next() % 8 {
        0 => TimeInForce::IOC,
        1 => TimeInForce::FOK,
        _ => TimeInForce::GTC,
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
    for ev in book.events() {
        if let Event::OrderInserted { handle, .. } = ev {
            live.push(*handle);
        }
    }
}

#[test]
fn scan_matches_bruteforce_under_random_flow() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(), 200_000, mid);
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let mut live: Vec<u32> = Vec::new();
    let mut oid = 1_u64;

    for step in 0..6000 {
        // Bias toward inserts early (build depth), then mix cancels.
        if !live.is_empty() && rng.next().is_multiple_of(3) {
            // Cancel a random tracked handle (may already be gone;
            // cancel_order returns false harmlessly).
            let k = (rng.next() as usize) % live.len();
            let h = live.swap_remove(k);
            book.cancel_order(h);
        } else {
            submit(&mut book, &mut live, &mut rng, mid, oid);
            oid += 1;
        }
        // Check every op for the first 200 (dense), then sample.
        if step < 200 || step % 7 == 0 {
            check(&book);
        }
    }
    check(&book);
}

#[test]
fn scan_matches_bruteforce_across_recenter() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(), 100_000, mid);
    let mut rng = Rng(0x0BAD_F00D_CAFE_1111);
    let mut live: Vec<u32> = Vec::new();
    let mut oid = 1_u64;

    for _ in 0..1500 {
        submit(&mut book, &mut live, &mut rng, mid, oid);
        oid += 1;
    }
    check(&book);

    // Drift mid up and recenter; migrate fully, then keep trading.
    let new_mid = mid + 30_000;
    book.trigger_recenter(new_mid);
    book.migrate_batch(10_000_000);
    check(&book);

    for step in 0..2000 {
        if !live.is_empty() && rng.next().is_multiple_of(3) {
            let k = (rng.next() as usize) % live.len();
            let h = live.swap_remove(k);
            book.cancel_order(h);
        } else {
            submit(&mut book, &mut live, &mut rng, new_mid, oid);
            oid += 1;
        }
        if step % 5 == 0 {
            check(&book);
        }
    }
    check(&book);
}
