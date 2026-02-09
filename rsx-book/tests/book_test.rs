use rsx_book::book::Orderbook;
use rsx_types::NONE;
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
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    assert_ne!(book.best_bid_tick, NONE);
}

#[test]
fn insert_ask_updates_best_ask() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    assert_ne!(book.best_ask_tick, NONE);
}

#[test]
fn insert_below_best_bid_no_change() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    let best = book.best_bid_tick;
    book.insert_resting(
        49_800, 100, Side::Buy, 0, 2, false, 0,
    );
    assert_eq!(book.best_bid_tick, best);
}

#[test]
fn cancel_updates_best_bid() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    let _h2 = book.insert_resting(
        49_800, 100, Side::Buy, 0, 2, false, 0,
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
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    book.cancel_order(h);
    assert_eq!(book.best_bid_tick, NONE);
}

#[test]
fn level_head_tail_count_qty_correct() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    let h2 = book.insert_resting(
        49_900, 200, Side::Buy, 0, 2, false, 0,
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
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    book.cancel_order(h);
    let h2 = book.insert_resting(
        49_900, 100, Side::Buy, 0, 2, false, 0,
    );
    assert_eq!(h2, h); // reused
}

#[test]
fn best_bid_less_than_best_ask() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 2, false, 0,
    );
    assert!(book.best_bid_tick < book.best_ask_tick);
}
