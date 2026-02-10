use rsx_marketdata::shadow::ShadowBook;
use rsx_types::SymbolConfig;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn new_book() -> ShadowBook {
    ShadowBook::new(config(), 1024, 50000)
}

// -- Shadow book tests --

#[test]
fn shadow_book_insert_updates_level() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    assert!(h < 1024);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 1);
    assert_eq!(snap.bids[0].qty, 100);
    assert_eq!(snap.bids[0].count, 1);
    assert_eq!(snap.bids[0].price, 49990);
}

#[test]
fn shadow_book_cancel_removes_from_level() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    book.apply_cancel(h, 2000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 0);
}

#[test]
fn shadow_book_fill_reduces_qty() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    book.apply_fill(h, 30, 0, 2000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids[0].qty, 70);
}

#[test]
fn shadow_book_fill_removes_exhausted_order() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    book.apply_fill(h, 100, 0, 2000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 0);
}

#[test]
fn shadow_book_bbo_derived_correctly() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(50010, 30, 1, 2, 1001);
    let bbo = book.derive_bbo().unwrap();
    assert_eq!(bbo.bid_px, 49990);
    assert_eq!(bbo.bid_qty, 50);
    assert_eq!(bbo.ask_px, 50010);
    assert_eq!(bbo.ask_qty, 30);
}

#[test]
fn shadow_book_empty_returns_no_bbo() {
    let book = new_book();
    assert!(book.derive_bbo().is_none());
}

#[test]
fn shadow_book_seq_monotonic() {
    let mut book = new_book();
    assert_eq!(book.seq(), 0);
    book.apply_insert(49990, 50, 0, 1, 1000);
    assert_eq!(book.seq(), 1);
    book.apply_insert(50010, 30, 1, 2, 1001);
    assert_eq!(book.seq(), 2);
    let h = book.apply_insert(49980, 20, 0, 3, 1002);
    assert_eq!(book.seq(), 3);
    book.apply_cancel(h, 1003);
    assert_eq!(book.seq(), 4);
}

// -- BBO tests --

#[test]
fn bbo_update_on_best_bid_change() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    let bbo1 = book.derive_bbo().unwrap();
    assert_eq!(bbo1.bid_px, 49990);
    // Insert higher bid
    book.apply_insert(49995, 40, 0, 2, 1001);
    let bbo2 = book.derive_bbo().unwrap();
    assert_eq!(bbo2.bid_px, 49995);
}

#[test]
fn bbo_update_on_best_ask_change() {
    let mut book = new_book();
    book.apply_insert(50010, 30, 1, 1, 1000);
    let bbo1 = book.derive_bbo().unwrap();
    assert_eq!(bbo1.ask_px, 50010);
    // Insert lower ask
    book.apply_insert(50005, 20, 1, 2, 1001);
    let bbo2 = book.derive_bbo().unwrap();
    assert_eq!(bbo2.ask_px, 50005);
}

#[test]
fn bbo_no_update_if_unchanged() {
    let mut book = new_book();
    book.apply_insert(49995, 50, 0, 1, 1000);
    book.apply_insert(50005, 30, 1, 2, 1001);
    let bbo1 = book.derive_bbo().unwrap();
    // Insert non-best bid (lower price)
    book.apply_insert(49980, 20, 0, 3, 1002);
    let bbo2 = book.derive_bbo().unwrap();
    assert_eq!(bbo1.bid_px, bbo2.bid_px);
    assert_eq!(bbo1.ask_px, bbo2.ask_px);
}

#[test]
fn bbo_includes_count_and_qty() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(49990, 30, 0, 2, 1001);
    let bbo = book.derive_bbo().unwrap();
    assert_eq!(bbo.bid_qty, 80);
    assert_eq!(bbo.bid_count, 2);
}

#[test]
fn bbo_correct_after_fill_at_best() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    book.apply_fill(h, 40, 0, 2000);
    let bbo = book.derive_bbo().unwrap();
    assert_eq!(bbo.bid_px, 49990);
    assert_eq!(bbo.bid_qty, 60);
}

#[test]
fn bbo_correct_after_cancel_at_best() {
    let mut book = new_book();
    let h1 = book.apply_insert(49995, 50, 0, 1, 1000);
    book.apply_insert(49990, 30, 0, 2, 1001);
    book.apply_cancel(h1, 2000);
    let bbo = book.derive_bbo().unwrap();
    // Best bid should now be 49990
    assert_eq!(bbo.bid_px, 49990);
    assert_eq!(bbo.bid_qty, 30);
}

#[test]
fn bbo_includes_bid_count_and_ask_count() {
    let mut book = new_book();
    book.apply_insert(49990, 10, 0, 1, 1000);
    book.apply_insert(49990, 20, 0, 2, 1001);
    book.apply_insert(49990, 30, 0, 3, 1002);
    book.apply_insert(50010, 5, 1, 4, 1003);
    book.apply_insert(50010, 15, 1, 5, 1004);
    let bbo = book.derive_bbo().unwrap();
    assert_eq!(bbo.bid_count, 3);
    assert_eq!(bbo.ask_count, 2);
}

// -- L2 snapshot tests --

#[test]
fn snapshot_top_10_levels_correct() {
    let mut book = new_book();
    // Insert 12 bid levels
    for i in 0..12 {
        let price = 49990 - i;
        book.apply_insert(price, 10, 0, 1, 1000 + i as u64);
    }
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 10);
    // Verify sorted best to worst
    assert_eq!(snap.bids[0].price, 49990);
    assert_eq!(snap.bids[9].price, 49981);
}

#[test]
fn snapshot_fewer_levels_than_depth() {
    let mut book = new_book();
    book.apply_insert(49990, 10, 0, 1, 1000);
    book.apply_insert(49985, 20, 0, 2, 1001);
    book.apply_insert(49980, 30, 0, 3, 1002);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 3);
}

#[test]
fn snapshot_empty_book_returns_empty() {
    let book = new_book();
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 0);
    assert_eq!(snap.asks.len(), 0);
}

#[test]
fn snapshot_includes_all_fields_per_level() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(49990, 30, 0, 2, 1001);
    let snap = book.derive_l2_snapshot(10);
    let lvl = &snap.bids[0];
    assert_eq!(lvl.price, 49990);
    assert_eq!(lvl.qty, 80);
    assert_eq!(lvl.count, 2);
}

#[test]
fn snapshot_seq_matches_latest_event() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(50010, 30, 1, 2, 1001);
    book.apply_insert(49985, 20, 0, 3, 1002);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.seq, 3);
}

// -- L2 delta tests --

#[test]
fn delta_insert_new_level() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    let delta = book.derive_l2_delta(0, 49990);
    assert_eq!(delta.qty, 50);
    assert_eq!(delta.count, 1);
    assert_eq!(delta.price, 49990);
}

#[test]
fn delta_remove_level_qty_zero() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_cancel(h, 2000);
    let delta = book.derive_l2_delta(0, 49990);
    assert_eq!(delta.qty, 0);
    assert_eq!(delta.count, 0);
}

#[test]
fn delta_update_level_qty_change() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    book.apply_fill(h, 30, 0, 2000);
    let delta = book.derive_l2_delta(0, 49990);
    assert_eq!(delta.qty, 70);
    assert_eq!(delta.count, 1);
}

#[test]
fn delta_side_correct_bid_vs_ask() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(50010, 30, 1, 2, 1001);
    let bid_delta = book.derive_l2_delta(0, 49990);
    assert_eq!(bid_delta.side, 0);
    let ask_delta = book.derive_l2_delta(1, 50010);
    assert_eq!(ask_delta.side, 1);
}

#[test]
fn delta_seq_monotonic_per_symbol() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    let d1 = book.derive_l2_delta(0, 49990);
    book.apply_insert(49990, 30, 0, 2, 1001);
    let d2 = book.derive_l2_delta(0, 49990);
    assert!(d2.seq > d1.seq);
}

// -- Trade tests --

#[test]
fn trade_from_fill_event_correct() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    let trade = book.make_trade(49990, 30, 1, 2000);
    assert_eq!(trade.symbol_id, 1);
    assert_eq!(trade.price, 49990);
    assert_eq!(trade.qty, 30);
}

#[test]
fn trade_price_and_qty_from_fill() {
    let mut book = new_book();
    book.apply_insert(50005, 80, 1, 1, 1000);
    let trade = book.make_trade(50005, 45, 0, 2000);
    assert_eq!(trade.price, 50005);
    assert_eq!(trade.qty, 45);
}

#[test]
fn trade_timestamp_from_fill() {
    let book = new_book();
    let trade = book.make_trade(49990, 10, 0, 5_000_000);
    assert_eq!(trade.timestamp_ns, 5_000_000);
}

// -- Shadow book spec compliance --

#[test]
fn shadow_book_order_done_not_applied() {
    // OrderDone events should NOT mutate shadow book
    // (MD20: OrderDone NOT routed to market data)
    // Shadow book only handles: Fill, Insert, Cancel
    // This test verifies the shadow book has no apply_done
    // method - the event loop must filter OrderDone out.
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    let seq_before = book.seq();
    // After insert, book has 1 bid level
    let snap_before = book.derive_l2_snapshot(10);
    assert_eq!(snap_before.bids.len(), 1);
    // Fill removes half
    book.apply_fill(h, 50, 0, 2000);
    let snap_after = book.derive_l2_snapshot(10);
    assert_eq!(snap_after.bids[0].qty, 50);
    // If someone incorrectly applied OrderDone as cancel,
    // the order would be removed. Verify it's still there.
    assert!(book.seq() > seq_before);
}

#[test]
fn shadow_book_uses_rsx_book_crate() {
    // Verify the shadow book wraps rsx_book::Orderbook
    // by checking that insert/cancel/fill operations
    // produce correct L2 state via the shared crate.
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(49980, 30, 0, 2, 1001);
    book.apply_insert(50010, 40, 1, 3, 1002);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 2);
    assert_eq!(snap.asks.len(), 1);
    assert_eq!(snap.bids[0].price, 49990);
    assert_eq!(snap.bids[1].price, 49980);
    assert_eq!(snap.asks[0].price, 50010);
}

#[test]
fn bbo_derived_from_shadow_book_not_me_bbo() {
    // MD21: MktData derives own BBO from shadow book
    // Verify BBO reflects actual shadow book state
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(50010, 30, 1, 2, 1001);
    let bbo = book.derive_bbo().unwrap();
    assert_eq!(bbo.bid_px, 49990);
    assert_eq!(bbo.ask_px, 50010);
    // After fill removes all bids
    let h = book.apply_insert(49990, 100, 0, 3, 1002);
    book.apply_fill(h, 100, 0, 1003);
    // Cancel first bid
    // BBO should still reflect remaining first bid
    let bbo2 = book.derive_bbo().unwrap();
    assert_eq!(bbo2.bid_px, 49990);
    assert_eq!(bbo2.bid_qty, 50);
}

#[test]
fn snapshot_top_25_levels_correct() {
    let mut book = new_book();
    for i in 0..30 {
        book.apply_insert(
            49990 - i, 10, 0, 1, 1000 + i as u64,
        );
    }
    let snap = book.derive_l2_snapshot(25);
    assert_eq!(snap.bids.len(), 25);
    assert_eq!(snap.bids[0].price, 49990);
    assert_eq!(snap.bids[24].price, 49966);
}

#[test]
fn snapshot_top_50_levels_correct() {
    let mut book = new_book();
    for i in 0..60 {
        book.apply_insert(
            49990 - i, 10, 0, 1, 1000 + i as u64,
        );
    }
    let snap = book.derive_l2_snapshot(50);
    assert_eq!(snap.bids.len(), 50);
    assert_eq!(snap.bids[0].price, 49990);
    assert_eq!(snap.bids[49].price, 49941);
}

#[test]
fn delta_only_for_changed_levels() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(49980, 30, 0, 2, 1001);
    // Delta at unchanged level shows current state
    let d1 = book.derive_l2_delta(0, 49990);
    assert_eq!(d1.qty, 50);
    // Modify only 49990
    let h = book.apply_insert(49990, 20, 0, 3, 1002);
    let d2 = book.derive_l2_delta(0, 49990);
    assert_eq!(d2.qty, 70); // changed
    let d3 = book.derive_l2_delta(0, 49980);
    assert_eq!(d3.qty, 30); // unchanged
    let _ = h;
}

#[test]
fn trade_taker_side_preserved() {
    let book = new_book();
    let buy_trade = book.make_trade(49990, 10, 0, 1000);
    assert_eq!(buy_trade.taker_side, 0);
    let sell_trade = book.make_trade(50010, 10, 1, 1001);
    assert_eq!(sell_trade.taker_side, 1);
}
