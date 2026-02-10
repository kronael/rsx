use rsx_book::book::Orderbook;
use rsx_book::event::CANCEL_POST_ONLY;
use rsx_book::event::Event;
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

fn incoming(
    price: i64,
    qty: i64,
    side: Side,
    post_only: bool,
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::GTC,
        user_id: 10,
        reduce_only: false,
        post_only,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 0,
    }
}

#[test]
fn post_only_buy_at_best_ask_cancelled() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1,
        false, 0, 0, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, true);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderCancelled { reason, .. }
            if *reason == CANCEL_POST_ONLY
    )));
}

#[test]
fn post_only_buy_above_best_ask_cancelled() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1,
        false, 0, 0, 0,
    );
    let mut order =
        incoming(50_200, 100, Side::Buy, true);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderCancelled { reason, .. }
            if *reason == CANCEL_POST_ONLY
    )));
}

#[test]
fn post_only_buy_below_best_ask_inserted() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1,
        false, 0, 0, 0,
    );
    let mut order =
        incoming(50_099, 100, Side::Buy, true);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
    assert!(!book.events().iter().any(|e| matches!(
        e,
        Event::OrderCancelled { .. }
    )));
}

#[test]
fn post_only_sell_at_best_bid_cancelled() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1,
        false, 0, 0, 0,
    );
    let mut order =
        incoming(49_900, 100, Side::Sell, true);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderCancelled { reason, .. }
            if *reason == CANCEL_POST_ONLY
    )));
}

#[test]
fn post_only_sell_below_best_bid_inserted() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1,
        false, 0, 0, 0,
    );
    let mut order =
        incoming(49_901, 100, Side::Sell, true);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
    assert!(!book.events().iter().any(|e| matches!(
        e,
        Event::OrderCancelled { .. }
    )));
}

#[test]
fn post_only_on_empty_book_inserted() {
    let mut book = test_book();
    let mut order =
        incoming(50_000, 100, Side::Buy, true);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

#[test]
fn non_post_only_crosses_fills_normally() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1,
        false, 0, 0, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, false);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::Fill { .. }
    )));
    assert!(!book.events().iter().any(|e| matches!(
        e,
        Event::OrderCancelled { .. }
    )));
}
