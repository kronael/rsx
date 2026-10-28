use crate::shadow::ShadowBook;
use rsx_types::SymbolConfig;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 10,
        price_decimals: 6,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

/// A ladder that migrates far from the book's creation anchor must keep
/// its price levels distinct: maybe_recenter re-anchors the compression
/// map (as the ME does), so the live ladder never lives in a compressed
/// zone.
#[test]
fn recenter_keeps_migrated_ladder_levels_distinct() {
    let mut book = ShadowBook::new(config(), 1024, 30_000);

    // Walk the market up 45% in 1% steps, quoting a 3-level ladder
    // (29 ticks apart) on each side of the moving mid, cancelling the
    // previous ladder each step — the demo maker's cancel-replace.
    let mut oid = 0u64;
    let mut live: Vec<u64> = Vec::new();
    let mut px = 30_000i64;
    while px < 43_500 {
        for old in live.drain(..) {
            assert!(
                book.apply_cancel_by_order_id(0, old, 1).is_some(),
                "cancel lost after recenter (oid {old})",
            );
        }
        for level in 0..3i64 {
            for (side, sign) in [(0u8, -1i64), (1u8, 1i64)] {
                oid += 1;
                let level_px = px + sign * (60 + level * 29);
                book.apply_insert_by_id(level_px, 1_000, side, 99, 1, 0, oid);
                book.maybe_recenter();
                live.push(oid);
            }
        }
        px = px * 101 / 100;
    }

    let snapshot = book.derive_l2_snapshot(10);
    assert_eq!(snapshot.bids.len(), 3, "bids aliased: {:?}", snapshot.bids);
    assert_eq!(snapshot.asks.len(), 3, "asks aliased: {:?}", snapshot.asks);
    for level in snapshot.bids.iter().chain(snapshot.asks.iter()) {
        assert_eq!(level.count, 1, "orders piled up: {level:?}");
        assert_eq!(level.qty, 1_000);
    }
}

/// A far ladder whose 29-tick levels share one compressed slot keeps
/// every raw price as its own L2 level.
#[test]
fn far_ladder_in_one_slot_keeps_raw_prices() {
    let mut book = ShadowBook::new(config(), 1024, 30_000);
    let mut oid = 0u64;
    for level in 0..3i64 {
        oid += 1;
        book.apply_insert_by_id(42_000 + level * 29, 1_000, 1, 99, 1, 0, oid);
    }
    let snapshot = book.derive_l2_snapshot(10);
    let asks: Vec<_> = snapshot
        .asks
        .iter()
        .map(|l| (l.price, l.qty, l.count))
        .collect();
    assert_eq!(
        asks,
        [(42_000, 1_000, 1), (42_029, 1_000, 1), (42_058, 1_000, 1)]
    );
}

/// One compressed slot holding both sides and several raw prices shows
/// each side's best price, qty and count in BBO and every raw price in L2.
#[test]
fn mixed_compressed_slot_shows_both_sides() {
    let mut book = ShadowBook::new(config(), 1024, 50_000);
    book.apply_insert_by_id(47_491, 10, 0, 1, 1, 0, 1);
    book.apply_insert_by_id(47_492, 7, 0, 1, 1, 0, 2);
    book.apply_insert_by_id(47_495, 5, 1, 2, 1, 0, 3);
    book.apply_insert_by_id(47_495, 2, 1, 2, 1, 0, 4);
    book.apply_insert_by_id(47_496, 3, 1, 2, 1, 0, 5);

    let bbo = book.derive_bbo().unwrap();
    assert_eq!((bbo.bid_px, bbo.bid_qty, bbo.bid_count), (47_492, 7, 1));
    assert_eq!((bbo.ask_px, bbo.ask_qty, bbo.ask_count), (47_495, 7, 2));

    let snapshot = book.derive_l2_snapshot(10);
    let bids: Vec<_> = snapshot
        .bids
        .iter()
        .map(|l| (l.price, l.qty, l.count))
        .collect();
    let asks: Vec<_> = snapshot
        .asks
        .iter()
        .map(|l| (l.price, l.qty, l.count))
        .collect();
    assert_eq!(bids, [(47_492, 7, 1), (47_491, 10, 1)]);
    assert_eq!(asks, [(47_495, 7, 2), (47_496, 3, 1)]);
}
