use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::FAIL_REDUCE_ONLY;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

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

fn user_net_qty(book: &Orderbook, user_id: u32) -> i64 {
    book.user_map
        .get(&user_id)
        .map(|&idx| {
            book.user_states[idx as usize].net_qty
        })
        .unwrap_or(0)
}

#[test]
fn fill_updates_taker_and_maker_net_qty() {
    let mut book = test_book();
    // Maker sell at 50100
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    // Taker buy crosses
    let mut order = IncomingOrder {
        price: 50_100,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);
    // Taker bought 100 -> net_qty = +100
    assert_eq!(user_net_qty(&book, 2), 100);
    // Maker sold 100 -> net_qty = -100
    assert_eq!(user_net_qty(&book, 1), -100);
}

#[test]
fn position_buy_increases_net_qty() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    let mut order = IncomingOrder {
        price: 50_100,
        qty: 50,
        remaining_qty: 50,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);
    assert_eq!(user_net_qty(&book, 2), 50);
}

#[test]
fn position_sell_decreases_net_qty() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 50, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    let mut order = IncomingOrder {
        price: 49_900,
        qty: 50,
        remaining_qty: 50,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);
    assert_eq!(user_net_qty(&book, 2), -50);
}

#[test]
fn user_state_assigned_on_first_order() {
    let mut book = test_book();
    assert!(!book.user_map.contains_key(&42));
    let mut order = IncomingOrder {
        price: 50_000,
        qty: 10,
        remaining_qty: 10,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 42,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);
    assert!(book.user_map.contains_key(&42));
}

#[test]
fn reduce_only_order_closes_long_position() {
    let mut book = test_book();
    // Build a long position for user 2: buy 100 from user 1
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    let mut buy = IncomingOrder {
        price: 50_100,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut buy);
    assert_eq!(user_net_qty(&book, 2), 100);

    // Now reduce-only sell to close
    book.insert_resting(
        50_100, 100, Side::Buy, 0, 3, false, 0, 0, 0,
    );
    let mut sell = IncomingOrder {
        price: 50_100,
        qty: 100,
        remaining_qty: 100,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: true,
        post_only: false,
        timestamp_ns: 2000,
        order_id_hi: 0,
        order_id_lo: 2,
    };
    process_new_order(&mut book, &mut sell);
    assert_eq!(user_net_qty(&book, 2), 0);
}

#[test]
fn reduce_only_order_clamped_to_position_size() {
    let mut book = test_book();
    // Build long 50 for user 2
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    let mut buy = IncomingOrder {
        price: 50_100,
        qty: 50,
        remaining_qty: 50,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut buy);
    assert_eq!(user_net_qty(&book, 2), 50);

    // Reduce-only sell 200 (should be clamped to 50)
    book.insert_resting(
        50_100, 200, Side::Buy, 0, 3, false, 0, 0, 0,
    );
    let mut sell = IncomingOrder {
        price: 50_100,
        qty: 200,
        remaining_qty: 200,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: true,
        post_only: false,
        timestamp_ns: 2000,
        order_id_hi: 0,
        order_id_lo: 2,
    };
    process_new_order(&mut book, &mut sell);
    // Clamped to 50, so net_qty should be 0
    assert_eq!(user_net_qty(&book, 2), 0);
}

#[test]
fn reduce_only_no_position_fails() {
    let mut book = test_book();
    // No position for user 5 -> reduce_only should fail
    let mut order = IncomingOrder {
        price: 50_000,
        qty: 10,
        remaining_qty: 10,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 5,
        reduce_only: true,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);
    let events = book.events();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::OrderFailed {
            reason: FAIL_REDUCE_ONLY,
            ..
        }
    ));
}
