//! Distribution correctness: drive the book through the real matching
//! path under each order-density SHAPE and cross-check every reachable
//! state against independent brute-force references.
//!
//! Shapes:
//! - dense: every level in a zone filled (packed book).
//! - sparse: a few levels scattered with large gaps across zones.
//! - concentrated: a wall of orders on one price level; heavy churn.
//! - adversarial: empty book, single level, full range spanning all
//!   zones, clustering on zone boundaries, and tick_size in {1,10,50}
//!   (exercises the COMPRESSION-ZONE-TICK-UNIT fix under real matching
//!   + a recenter/migrate).
//!
//! Every check is derived from the book independently of the fast path:
//! - `ref_bid`/`ref_ask`: brute-force max-BUY-head / min-SELL-head over
//!   the whole slot array (the pre-bitmap O(slots) semantics).
//! - bitmap <=> `order_count`: the set of set bits (walked via
//!   `find_next`) must equal the set of non-empty levels of that side.
//! - uncrossed: read the head prices at the reference ticks directly.
//! - slab no-leak (invariant #8): `slab.len() == free_count() + active`,
//!   plus every active handle appears in exactly one level chain.

use rsx_book::book::BookState;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::occupancy::Occupancy;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rsx_types::NONE;
use std::collections::HashSet;

fn config(tick: i64) -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: tick,
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

/// The set of slot indices whose bit is set, walked independently of the
/// scan path via repeated `find_next`.
fn occ_set(occ: &Occupancy) -> Vec<u32> {
    let mut v = Vec::new();
    let mut from = 0u32;
    while let Some(b) = occ.find_next(from) {
        v.push(b);
        from = b + 1;
    }
    v
}

/// The set of non-empty levels whose HEAD is `want_buy` — the reference
/// the occupancy bitmap must reproduce exactly.
fn level_occ(book: &Orderbook, want_buy: bool) -> Vec<u32> {
    let mut v = Vec::new();
    for (i, lvl) in book.active_levels.iter().enumerate() {
        if lvl.order_count == 0 {
            continue;
        }
        let is_buy =
            book.orders.get(lvl.head).side == Side::Buy as u8;
        if is_buy == want_buy {
            v.push(i as u32);
        }
    }
    v
}

/// Invariant #8 (slab no-leak) + level-chain integrity. Every active
/// handle appears in exactly one level's doubly-linked list, each chain
/// length equals its `order_count`, and allocated == free + active.
/// Only valid in `Normal` state (during migration orders also live in
/// `old_levels`, which this does not walk).
fn check_slab_noleak(book: &Orderbook) {
    let mut seen: HashSet<u32> = HashSet::new();
    for lvl in book.active_levels.iter() {
        if lvl.order_count == 0 {
            continue;
        }
        let mut cur = lvl.head;
        let mut walked = 0u32;
        while cur != NONE {
            assert!(
                seen.insert(cur),
                "slab alias: handle {} linked in two chains",
                cur,
            );
            assert!(
                book.orders.get(cur).is_active(),
                "inactive order {} still linked",
                cur,
            );
            walked += 1;
            cur = book.orders.get(cur).next;
        }
        assert_eq!(
            walked, lvl.order_count,
            "level chain length != order_count",
        );
    }
    let active = seen.len() as u32;
    assert_eq!(
        book.orders.len(),
        book.orders.free_count() + active,
        "slab leak: len {} != free {} + active {}",
        book.orders.len(),
        book.orders.free_count(),
        active,
    );
}

/// Full invariant sweep. Fast scan == brute force, tracked BBA == scan,
/// bitmap == level occupancy, book uncrossed (by head price), slab
/// no-leak. Call only in `Normal` state.
fn check(book: &Orderbook) {
    assert_eq!(book.state, BookState::Normal, "check in Migrating");
    let rb = ref_bid(book);
    let ra = ref_ask(book);
    assert_eq!(book.scan_next_bid(NONE), rb, "bid scan diverged");
    assert_eq!(book.scan_next_ask(NONE), ra, "ask scan diverged");
    assert_eq!(book.best_bid_tick, rb, "best_bid_tick stale");
    assert_eq!(book.best_ask_tick, ra, "best_ask_tick stale");

    assert_eq!(
        occ_set(&book.bid_occ),
        level_occ(book, true),
        "bid bitmap != level occupancy",
    );
    assert_eq!(
        occ_set(&book.ask_occ),
        level_occ(book, false),
        "ask bitmap != level occupancy",
    );

    if rb != NONE && ra != NONE {
        let bid_px =
            book.orders.get(book.active_levels[rb as usize].head).price.0;
        let ask_px =
            book.orders.get(book.active_levels[ra as usize].head).price.0;
        assert!(
            bid_px < ask_px,
            "crossed book: bid {} >= ask {}",
            bid_px,
            ask_px,
        );
    }
    check_slab_noleak(book);
}

fn rest(
    book: &mut Orderbook,
    buy: bool,
    price: i64,
    qty: i64,
    oid: u64,
) -> u32 {
    let side = if buy { Side::Buy } else { Side::Sell };
    book.insert_resting(price, qty, side, 0, 1, false, 1, 0, oid)
}

/// Aggressor through the real matching path.
fn taker(
    book: &mut Orderbook,
    buy: bool,
    price: i64,
    qty: i64,
    tif: TimeInForce,
    oid: u64,
) {
    let mut o = IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side: if buy { Side::Buy } else { Side::Sell },
        tif,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: oid,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut o);
}

// --- dense: every level in zone 0 filled (packed book) ----------------

#[test]
fn dense_packed_next_best_and_cancel() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(1), 20_000, mid);
    let n = 2_000_i64;
    let mut bids: Vec<u32> = Vec::new();
    for i in 1..=n {
        bids.push(rest(&mut book, true, mid - i, 10, i as u64));
        rest(&mut book, false, mid + i, 10, (100_000 + i) as u64);
    }
    check(&book);
    assert_eq!(book.best_bid_px, mid - 1);
    assert_eq!(book.best_ask_px, mid + 1);

    // Clear the touch ask exactly; next-best steps one tick out.
    taker(&mut book, true, mid + 1, 10, TimeInForce::IOC, 1);
    assert_eq!(book.best_ask_px, mid + 2);
    check(&book);

    // Cancel the touch bid; next-best steps one tick in-book.
    book.cancel_order(bids[0]);
    assert_eq!(book.best_bid_px, mid - 2);
    check(&book);

    // Sweep eight contiguous ask levels in one taker.
    taker(&mut book, true, mid + 100, 80, TimeInForce::IOC, 2);
    assert_eq!(book.best_ask_px, mid + 10);
    check(&book);
}

// --- sparse: few levels, large gaps across zones ----------------------

#[test]
fn sparse_gaps_across_zones() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(1), 1_000, mid);
    // Offsets landing in zone 0 / 1 / 3 / 4 with big empty gaps between.
    let offs = [1_i64, 300, 8_000, 120_000, 400_000, 900_000];
    let mut bids: Vec<u32> = Vec::new();
    for (k, &o) in offs.iter().enumerate() {
        bids.push(rest(&mut book, true, mid - o, 10, k as u64));
        rest(&mut book, false, mid + o, 10, (100 + k) as u64);
    }
    check(&book);
    assert_eq!(book.best_bid_px, mid - 1);
    assert_eq!(book.best_ask_px, mid + 1);

    // Clear nearest ask; best jumps the gap to +300.
    taker(&mut book, true, mid + 1, 10, TimeInForce::IOC, 1);
    assert_eq!(book.best_ask_px, mid + 300);
    check(&book);

    // Cancel nearest bid; best jumps to -300.
    book.cancel_order(bids[0]);
    assert_eq!(book.best_bid_px, mid - 300);
    check(&book);

    // Sweep four crossing ask levels (each qty 10) across the gaps.
    taker(&mut book, true, mid + 500_000, 40, TimeInForce::IOC, 2);
    assert_eq!(book.best_ask_px, mid + 900_000);
    check(&book);
}

// --- concentrated: a wall of orders on one level, heavy churn ----------

#[test]
fn concentrated_wall_heavy_churn() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(1), 4_000, mid);
    let mut bidwall: Vec<u32> = Vec::new();
    for i in 0..200u64 {
        bidwall.push(rest(&mut book, true, mid - 5, 1, i));
        rest(&mut book, false, mid + 5, 1, 1_000 + i);
    }
    check(&book);
    assert_eq!(
        book.active_levels[book.best_bid_tick as usize].order_count,
        200,
    );

    let mut oid = 10_000u64;
    for _ in 0..50u64 {
        // Cancel 10 from the bid wall, refill 10 (net-neutral count).
        for _ in 0..10 {
            if let Some(h) = bidwall.pop() {
                book.cancel_order(h);
            }
        }
        for _ in 0..10 {
            bidwall.push(rest(&mut book, true, mid - 5, 1, oid));
            oid += 1;
        }
        // Partial-fill three off the ask wall (single price -> no stale
        // best), then refill them.
        taker(&mut book, true, mid + 5, 3, TimeInForce::IOC, oid);
        oid += 1;
        for _ in 0..3 {
            rest(&mut book, false, mid + 5, 1, oid);
            oid += 1;
        }
        check(&book);
    }

    // Fully clear the ask wall in one shot: best ask -> NONE.
    let ask_count =
        book.active_levels[book.best_ask_tick as usize].order_count as i64;
    taker(&mut book, true, mid + 5, ask_count, TimeInForce::IOC, oid);
    assert_eq!(book.best_ask_tick, NONE);
    check(&book);
}

// --- adversarial -------------------------------------------------------

#[test]
fn adversarial_empty_and_single_level() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(1), 100, mid);
    // Empty book: no best, scans NONE, taker against nothing is safe.
    check(&book);
    assert_eq!(book.best_bid_tick, NONE);
    assert_eq!(book.best_ask_tick, NONE);
    taker(&mut book, true, mid + 1, 10, TimeInForce::IOC, 1);
    check(&book);
    assert_eq!(book.best_ask_tick, NONE);

    // One order each side.
    let b = rest(&mut book, true, mid - 1, 10, 2);
    let a = rest(&mut book, false, mid + 1, 10, 3);
    check(&book);
    assert_eq!(book.best_bid_px, mid - 1);
    assert_eq!(book.best_ask_px, mid + 1);

    // Clear both -> empty again.
    book.cancel_order(b);
    book.cancel_order(a);
    assert_eq!(book.best_bid_tick, NONE);
    assert_eq!(book.best_ask_tick, NONE);
    check(&book);
}

#[test]
fn adversarial_full_range_all_zones() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(1), 4_000, mid);
    // One order per representative offset in every zone, both sides.
    let offs = [
        1_i64, 20_000, 49_999, // zone 0
        60_000, 140_000, // zone 1
        160_000, 290_000, // zone 2
        320_000, 480_000, // zone 3
        700_000, // zone 4
    ];
    for (k, &o) in offs.iter().enumerate() {
        rest(&mut book, true, mid - o, 10, k as u64);
        rest(&mut book, false, mid + o, 10, (500 + k) as u64);
    }
    check(&book);
    assert_eq!(book.best_bid_px, mid - 1);
    assert_eq!(book.best_ask_px, mid + 1);

    // A big taker crossing every ask up to zone 3 (nine levels * 10);
    // next-best walks zones in true price order despite the sawtooth.
    taker(&mut book, true, mid + 490_000, 1_000, TimeInForce::IOC, 1);
    assert_eq!(book.best_ask_px, mid + 700_000);
    check(&book);
}

#[test]
fn adversarial_zone_boundary_clustering() {
    let mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(1), 4_000, mid);
    // Clean near-mid touch so BBA is always a single-price zone-0 level.
    rest(&mut book, true, mid - 1, 10, 1);
    rest(&mut book, false, mid + 1, 10, 2);
    // Cluster orders straddling every raw threshold (Part 1: these must
    // map into real zone slots, not collapse into the zone-4 catch-all).
    let th = book.compression.thresholds;
    let mut far: Vec<u32> = Vec::new();
    let mut oid = 100u64;
    for &t in th.iter() {
        for d in [-2i64, -1, 0, 1, 2] {
            far.push(rest(&mut book, true, mid - (t + d), 10, oid));
            oid += 1;
            far.push(rest(&mut book, false, mid + (t + d), 10, oid));
            oid += 1;
        }
    }
    check(&book);
    assert_eq!(book.best_bid_px, mid - 1);
    assert_eq!(book.best_ask_px, mid + 1);

    // Cancel every far order; BBA untouched, scans stay correct.
    for h in far {
        book.cancel_order(h);
    }
    check(&book);
    assert_eq!(book.best_bid_px, mid - 1);
    assert_eq!(book.best_ask_px, mid + 1);
}

#[test]
fn adversarial_tick_sizes_matching_and_recenter() {
    for &tick in &[1_i64, 10, 50] {
        let mid = 1_000_000_i64;
        let mut book = Orderbook::new(config(tick), 20_000, mid);

        // Fat, tick-aligned book spanning zone 0 out to zone 2. Bids and
        // asks stay separated by a spread so nothing crosses on rest.
        let mut bids: Vec<u32> = Vec::new();
        let mut oid = 1u64;
        for step in 1..=400i64 {
            let off = step * tick * 5; // tick-aligned, fans out
            bids.push(rest(&mut book, true, mid - off, 10, oid));
            oid += 1;
            rest(&mut book, false, mid + off, 10, oid);
            oid += 1;
        }
        check(&book);

        // should_recenter uses raw drift vs a raw zone-0 half-width
        // (Part 1 also fixes migration.rs). A drift just over half of
        // zone-0 width must trip; a tiny drift must not.
        let z0_half = book.compression.thresholds[0];
        assert!(!book.should_recenter(mid + tick));
        assert!(book.should_recenter(mid + z0_half / 2 + tick));

        // Matching takers that clear the touch, tick-aligned prices.
        for k in 0..20i64 {
            let px = mid + (k + 1) * tick * 5;
            taker(&mut book, true, px, 30, TimeInForce::IOC, oid);
            oid += 1;
            check(&book);
        }
        // Cancel a handful of touch bids too.
        for _ in 0..20 {
            if let Some(h) = bids.pop() {
                book.cancel_order(h);
            }
        }
        check(&book);

        // Recenter up and migrate to completion, then keep trading.
        // new_mid is deliberately OFF any resting level (offsets are
        // multiples of 5*tick; 503*tick is not) — migration orphans an
        // order resting exactly at new_mid (MIGRATE-SKIPS-NEW-MID-LEVEL
        // in bugs.md), which would trip the slab no-leak check.
        let new_mid = mid + 503 * tick;
        book.trigger_recenter(new_mid);
        book.migrate_batch(50_000_000);
        assert_eq!(book.state, BookState::Normal, "migration incomplete");
        check(&book);

        for k in 0..20i64 {
            let px = new_mid + (k + 1) * tick * 5;
            taker(&mut book, true, px, 20, TimeInForce::IOC, oid);
            oid += 1;
            check(&book);
        }
        check(&book);
    }
}
