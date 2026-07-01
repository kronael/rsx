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

// --- BOOK-BBO-COMPRESSED-INDEX regression tests ---
//
// The compression map is a sawtooth: tick index is NOT globally
// price-monotonic. With mid=50_000, price 45_000 lands at a HIGHER tick
// index than price 49_900, even though 49_900 is the better bid. BBA
// must be chosen by raw price, not by comparing tick indices.

fn price_at_best_bid(book: &Orderbook) -> i64 {
    let lvl = &book.active_levels
        [book.best_bid_tick as usize];
    book.orders.get(lvl.head).price.0
}

fn price_at_best_ask(book: &Orderbook) -> i64 {
    let lvl = &book.active_levels
        [book.best_ask_tick as usize];
    book.orders.get(lvl.head).price.0
}

#[test]
fn best_bid_is_highest_price_across_zones() {
    let mut book = test_book();
    // 45_000 sits in a deeper compression zone -> higher tick index
    // than 49_900, but 49_900 is the better (higher) bid.
    let deep_tick =
        book.compression.price_to_index(45_000);
    let near_tick =
        book.compression.price_to_index(49_900);
    assert!(
        deep_tick > near_tick,
        "precondition: sawtooth (deep {} > near {})",
        deep_tick,
        near_tick,
    );
    // Insert the deep (worse) bid first, then the near (better) bid.
    book.insert_resting(
        45_000, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    assert_eq!(
        price_at_best_bid(&book),
        49_900,
        "best bid must be the highest price, not highest tick",
    );
}

#[test]
fn best_ask_is_lowest_price() {
    // Non-regression: the ask half of the compression map happens to be
    // globally index-monotonic (base indices grow outward and the ask
    // half adds local_offset with distance), so the original index
    // comparison did NOT misbehave for asks. This test does not fail on
    // the buggy code; it guards that the price-based rewrite still picks
    // the lowest-priced ask. The bid-side and crossing tests carry the
    // fail-before/pass-after evidence.
    let mut book = test_book();
    book.insert_resting(
        55_000, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 2, false, 0, 0, 0,
    );
    assert_eq!(
        price_at_best_ask(&book),
        50_100,
        "best ask must be the lowest price",
    );
}

#[test]
fn post_only_sell_crossing_bid_across_zones_cancelled() {
    // Crossing detection across compression zones. Resting best bid at
    // 49_900 (near, low tick). A post_only SELL at 45_000 truly crosses
    // (45_000 <= 49_900) and must be CANCELLED. The old index-based
    // check compared ticks: sell tick (5249, deep zone) > bid tick
    // (2399), so it wrongly saw "no cross" and would rest the order --
    // producing a crossed book. This asserts the price-based fix.
    use rsx_book::event::CANCEL_POST_ONLY;
    use rsx_book::event::Event;
    use rsx_book::matching::IncomingOrder;
    use rsx_book::matching::process_new_order;
    use rsx_types::TimeInForce;

    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let bid_tick = book.best_bid_tick;
    let sell_tick =
        book.compression.price_to_index(45_000);
    assert!(
        sell_tick > bid_tick,
        "precondition: sawtooth (sell tick {} > bid tick {})",
        sell_tick,
        bid_tick,
    );
    let mut order = IncomingOrder {
        price: 45_000,
        qty: 50,
        remaining_qty: 50,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 9,
        reduce_only: false,
        post_only: true,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 0,
    };
    process_new_order(&mut book, &mut order);
    assert!(
        book.events().iter().any(|e| matches!(
            e,
            Event::OrderCancelled { reason, .. }
                if *reason == CANCEL_POST_ONLY
        )),
        "post_only sell crossing the bid must be cancelled",
    );
}

// --- BOOK-SCAN-NEXT-BID-OFFBY regression ---

#[test]
fn cancel_best_bid_at_tick_one_keeps_tick_zero() {
    let mut book = test_book();
    // Construct two bid prices that land at compression ticks 1 and 0.
    let mut px_tick0 = 0i64;
    let mut px_tick1 = 0i64;
    for p in (1..=50_000i64).rev() {
        let t = book.compression.price_to_index(p);
        if t == 0 && px_tick0 == 0 {
            px_tick0 = p;
        }
        if t == 1 && px_tick1 == 0 {
            px_tick1 = p;
        }
        if px_tick0 != 0 && px_tick1 != 0 {
            break;
        }
    }
    assert!(
        px_tick0 != 0 && px_tick1 != 0,
        "found tick0 px={} tick1 px={}",
        px_tick0,
        px_tick1,
    );
    // Rest a bid at tick 0 and a (better) bid at tick 1.
    book.insert_resting(
        px_tick0, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let h1 = book.insert_resting(
        px_tick1, 100, Side::Buy, 0, 2, false, 0, 0, 0,
    );
    // Cancel the best bid at tick 1; tick 0's bid must survive as BBA.
    book.cancel_order(h1);
    assert_ne!(
        book.best_bid_tick, NONE,
        "cancelling tick-1 best must not drop the tick-0 bid",
    );
    assert_eq!(
        book.best_bid_tick, 0,
        "tick 0 must become the new best bid",
    );
    assert_eq!(price_at_best_bid(&book), px_tick0);
}
