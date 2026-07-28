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
/// zone where prices alias into one slot.
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

/// Without recentering, the same walk aliases the far ladder.
#[test]
fn no_recenter_aliases_far_ladder() {
    let mut book = ShadowBook::new(config(), 1024, 30_000);
    let mut oid = 0u64;
    // Park a 3-level ask ladder 40% above the anchor (zone 3).
    for level in 0..3i64 {
        oid += 1;
        book.apply_insert_by_id(42_000 + level * 29, 1_000, 1, 99, 1, 0, oid);
    }
    let snapshot = book.derive_l2_snapshot(10);
    assert!(
        snapshot.asks.len() < 3,
        "zone 3 no longer aliases 29-tick levels: {:?}",
        snapshot.asks,
    );
}
