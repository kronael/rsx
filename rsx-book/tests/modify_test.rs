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
fn modify_price_cancels_and_reinserts() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    // Extra order so slab doesn't reuse h1 slot
    let _h_extra = book.insert_resting(
        49_800, 50, Side::Buy, 0, 2, false,
        0, 0, 0,
    );
    let h2 = book.modify_order_price(
        h1, 49_700, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    assert!(book.orders.get(h2).is_active());
    assert_eq!(
        book.orders.get(h2).price, Price(49_700)
    );
    assert_eq!(
        book.orders.get(h2).remaining_qty, Qty(100)
    );
    // Old tick level should be empty
    let old_tick =
        book.compression.price_to_index(49_900);
    assert_eq!(
        book.active_levels[old_tick as usize]
            .order_count,
        0,
    );
}

#[test]
fn modify_price_loses_time_priority() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    let h2 = book.insert_resting(
        49_900, 200, Side::Buy, 0, 2, false,
        0, 0, 0,
    );
    // Move h1 to same price -- goes behind h2
    let h3 = book.modify_order_price(
        h1, 49_900, Side::Buy, 0, 1, false,
        1, 0, 0,
    );
    let tick =
        book.compression.price_to_index(49_900);
    let level =
        &book.active_levels[tick as usize];
    assert_eq!(level.head, h2);
    assert_eq!(level.tail, h3);
}

#[test]
fn modify_qty_down_in_place() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    assert!(book.modify_order_qty_down(h, 60));
    assert_eq!(
        book.orders.get(h).remaining_qty, Qty(60)
    );
}

#[test]
fn modify_qty_down_keeps_time_priority() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    let h2 = book.insert_resting(
        49_900, 200, Side::Buy, 0, 2, false,
        0, 0, 0,
    );
    book.modify_order_qty_down(h1, 50);
    let tick =
        book.compression.price_to_index(49_900);
    let level =
        &book.active_levels[tick as usize];
    assert_eq!(level.head, h1);
    assert_eq!(level.tail, h2);
}

#[test]
fn modify_qty_down_updates_level_total_qty() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
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
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    assert!(book.modify_order_qty_down(h, 0));
    assert!(!book.orders.get(h).is_active());
    assert_eq!(book.best_bid_tick, NONE);
}
