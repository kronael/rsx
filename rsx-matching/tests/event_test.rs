use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderFailureReason;
use rsx_book::event::MAX_EVENTS;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
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
fn event_buffer_reserves_capacity_at_construction() {
    let book = test_book();
    assert!(book.event_buf.capacity() >= MAX_EVENTS);
    assert!(book.event_buf.is_empty());
}

#[test]
fn emit_writes_sequential_slots() {
    let mut book = test_book();
    book.emit(Event::OrderFailed {
        user_id: 1,
        reason: OrderFailureReason::InvalidTick,
        order_id_hi: 0,
        order_id_lo: 0,
    });
    book.emit(Event::OrderFailed {
        user_id: 2,
        reason: OrderFailureReason::InvalidTick,
        order_id_hi: 0,
        order_id_lo: 0,
    });
    assert_eq!(book.events().len(), 2);
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
fn event_buffer_clears_per_cycle() {
    let mut book = test_book();
    book.emit(Event::OrderFailed {
        user_id: 1,
        reason: OrderFailureReason::InvalidTick,
        order_id_hi: 0,
        order_id_lo: 0,
    });
    assert_eq!(book.events().len(), 1);

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
    let events = book.events();
    assert!(!events.is_empty());
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::OrderFailed { user_id: 1, .. })));
}
