use rsx_book::book::Orderbook;
use rsx_types::NONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;


fn test_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn test_book() -> Orderbook {
    Orderbook::new(test_config(), 1024, 50_000)
}

#[test]
fn insert_bid_updates_best_bid() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    assert_ne!(book.best_bid_tick, NONE);
}

#[test]
fn insert_ask_updates_best_ask() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    assert_ne!(book.best_ask_tick, NONE);
}

#[test]
fn insert_below_best_bid_no_change() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let best = book.best_bid_tick;
    book.insert_resting(
        49_800, 100, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    assert_eq!(book.best_bid_tick, best);
}

#[test]
fn cancel_updates_best_bid() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let _h2 = book.insert_resting(
        49_800, 100, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    let old_best = book.best_bid_tick;
    book.cancel_order(h1);
    assert_ne!(book.best_bid_tick, old_best);
    assert_ne!(book.best_bid_tick, NONE);
}

#[test]
fn empty_book_after_all_cancels() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.cancel_order(h);
    assert_eq!(book.best_bid_tick, NONE);
}

#[test]
fn level_head_tail_count_qty_correct() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let h2 = book.insert_resting(
        49_900, 200, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    let tick =
        book.compression.price_to_index(49_900);
    let level = &book.active_levels[tick as usize];
    assert_eq!(level.head, h1);
    assert_eq!(level.tail, h2);
    assert_eq!(level.order_count, 2);
    assert_eq!(level.total_qty, 300);
}

#[test]
fn slab_reuse_after_cancel() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.cancel_order(h);
    let h2 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    assert_eq!(h2, h); // reused
}

#[test]
fn best_bid_less_than_best_ask() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 2, false, 0, 0, 0,
    );
    assert!(book.best_bid_tick < book.best_ask_tick);
}

#[test]
fn modify_price_cancels_and_reinserts() {
    let mut book = test_book();
    // Insert two orders so slab won't reuse h1 as h2
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let _h_extra = book.insert_resting(
        49_800, 50, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    let h2 = book.modify_order_price(
        h1, 49_700, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    // h1 was freed and reused as h2 (slab reuse)
    // Just verify the new order has correct state
    assert!(book.orders.get(h2).is_active());
    assert_eq!(book.orders.get(h2).price, Price(49_700));
    assert_eq!(
        book.orders.get(h2).remaining_qty,
        Qty(100)
    );
    // Old tick level should have lost the order
    let old_tick =
        book.compression.price_to_index(49_900);
    assert_eq!(
        book.active_levels[old_tick as usize].order_count,
        0,
    );
}

#[test]
fn modify_price_loses_time_priority() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let h2 = book.insert_resting(
        49_900, 200, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    // Move h1 to same price -- should go behind h2
    let h3 = book.modify_order_price(
        h1, 49_900, Side::Buy, 0, 1, false, 1,
        0, 0,
    );
    let tick =
        book.compression.price_to_index(49_900);
    let level = &book.active_levels[tick as usize];
    assert_eq!(level.head, h2);
    assert_eq!(level.tail, h3);
}

#[test]
fn modify_qty_down_in_place() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    assert!(book.modify_order_qty_down(h, 60));
    assert_eq!(book.orders.get(h).remaining_qty, Qty(60));
}

#[test]
fn modify_qty_down_keeps_time_priority() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let h2 = book.insert_resting(
        49_900, 200, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    book.modify_order_qty_down(h1, 50);
    let tick =
        book.compression.price_to_index(49_900);
    let level = &book.active_levels[tick as usize];
    // h1 still at head (time priority preserved)
    assert_eq!(level.head, h1);
    assert_eq!(level.tail, h2);
}

#[test]
fn modify_qty_down_updates_level_total_qty() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.modify_order_qty_down(h, 40);
    let tick =
        book.compression.price_to_index(49_900);
    assert_eq!(
        book.active_levels[tick as usize].total_qty,
        40,
    );
}

#[test]
fn modify_qty_down_to_zero_removes_order() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    assert!(book.modify_order_qty_down(h, 0));
    assert!(!book.orders.get(h).is_active());
    assert_eq!(book.best_bid_tick, NONE);
}
