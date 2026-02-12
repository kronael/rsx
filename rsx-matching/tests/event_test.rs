use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::MAX_EVENTS;
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

#[test]
fn event_buffer_fixed_array_no_heap() {
    let book = test_book();
    // event_buf is [Event; MAX_EVENTS], stack-allocated
    // in the Orderbook struct. Verify size.
    assert_eq!(book.event_buf.len(), MAX_EVENTS);
    assert_eq!(book.event_len, 0);
}

#[test]
fn emit_writes_sequential_slots() {
    let mut book = test_book();
    book.emit(Event::OrderFailed {
        user_id: 1,
        reason: 0,
    });
    book.emit(Event::OrderFailed {
        user_id: 2,
        reason: 0,
    });
    assert_eq!(book.event_len, 2);
    match book.events()[0] {
        Event::OrderFailed { user_id, .. } => {
            assert_eq!(user_id, 1);
        }
        _ => panic!("expected OrderFailed"),
    }
    match book.events()[1] {
        Event::OrderFailed { user_id, .. } => {
            assert_eq!(user_id, 2);
        }
        _ => panic!("expected OrderFailed"),
    }
}

#[test]
fn event_buffer_max_10000_events() {
    assert_eq!(MAX_EVENTS, 10_000);
}

#[test]
fn event_len_reset_per_cycle_single_store() {
    let mut book = test_book();
    book.emit(Event::OrderFailed {
        user_id: 1,
        reason: 0,
    });
    assert_eq!(book.event_len, 1);

    // process_new_order resets event_len to 0 at start
    let mut order = IncomingOrder {
        price: 50_000,
        qty: 10,
        remaining_qty: 10,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);
    // Old event gone; new events from this cycle only
    let events = book.events();
    assert!(events.len() >= 1);
    // No leftover from previous cycle
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::OrderFailed { user_id: 1, .. }
    )));
}

#[test]
fn fills_precede_order_done_always() {
    let mut book = test_book();
    // Resting sell, aggressive buy -> fill then done
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
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

    let events = book.events();
    let mut saw_fill = false;
    for event in events {
        match event {
            Event::Fill { .. } => {
                saw_fill = true;
            }
            Event::OrderDone { .. } => {
                assert!(
                    saw_fill,
                    "OrderDone before any Fill"
                );
            }
            _ => {}
        }
    }
    assert!(saw_fill, "expected at least one fill");
}

#[test]
fn exactly_one_completion_per_order() {
    let mut book = test_book();
    // Full fill: taker fully matched
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
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

    let events = book.events();
    // Count completions for taker (user 2)
    let taker_dones = events.iter().filter(|e| {
        matches!(
            e,
            Event::OrderDone { user_id: 2, .. }
        )
    }).count();
    assert_eq!(
        taker_dones, 1,
        "taker should have exactly one OrderDone"
    );

    // Count completions for maker (user 1)
    let maker_dones = events.iter().filter(|e| {
        matches!(
            e,
            Event::OrderDone { user_id: 1, .. }
        )
    }).count();
    assert_eq!(
        maker_dones, 1,
        "maker should have exactly one OrderDone"
    );
}
