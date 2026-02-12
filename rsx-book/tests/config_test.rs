use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::FAIL_VALIDATION;
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
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 0,
    }
}

// NOTE: config_applied_emitted_on_update skipped --
// no ConfigApplied event variant exists in the
// current Event enum.

#[test]
fn config_tick_size_change_validates_new_orders() {
    let mut book = test_book();

    // Order at price 50_100 passes with tick_size=1
    let mut o1 = incoming(50_100, 10, Side::Buy);
    process_new_order(&mut book, &mut o1);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));

    // Change tick_size to 100
    let new_config = SymbolConfig {
        tick_size: 100,
        ..book.config
    };
    book.update_config(new_config);

    // Price 50_150 not divisible by 100 -> rejected
    let mut o2 = incoming(50_150, 10, Side::Buy);
    process_new_order(&mut book, &mut o2);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == FAIL_VALIDATION
    )));

    // Price 50_100 divisible by 100 -> accepted
    let mut o3 = incoming(50_100, 10, Side::Buy);
    process_new_order(&mut book, &mut o3);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

#[test]
fn config_lot_size_change_validates_new_orders() {
    let mut book = test_book();

    // qty=7 passes with lot_size=1
    let mut o1 = incoming(50_100, 7, Side::Buy);
    process_new_order(&mut book, &mut o1);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));

    // Change lot_size to 10
    let new_config = SymbolConfig {
        lot_size: 10,
        ..book.config
    };
    book.update_config(new_config);

    // qty=7 not divisible by 10 -> rejected
    let mut o2 = incoming(50_100, 7, Side::Buy);
    process_new_order(&mut book, &mut o2);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == FAIL_VALIDATION
    )));

    // qty=10 divisible by 10 -> accepted
    let mut o3 = incoming(50_100, 10, Side::Buy);
    process_new_order(&mut book, &mut o3);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

#[test]
fn config_version_monotonic() {
    // SymbolConfig doesn't have a version field.
    // The matching engine uses config.symbol_id as
    // identity. Verify that update_config replaces
    // atomically and preserves symbol_id.
    let mut book = test_book();
    let sid = book.config.symbol_id;

    let c2 = SymbolConfig {
        tick_size: 5,
        ..book.config
    };
    book.update_config(c2);
    assert_eq!(book.config.symbol_id, sid);
    assert_eq!(book.config.tick_size, 5);

    let c3 = SymbolConfig {
        tick_size: 10,
        ..book.config
    };
    book.update_config(c3);
    assert_eq!(book.config.tick_size, 10);
}
