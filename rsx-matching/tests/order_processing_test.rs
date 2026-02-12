use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::FAIL_VALIDATION;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_matching::dedup::DedupTracker;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use std::time::Duration;
use std::time::Instant;

fn test_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 10,
        lot_size: 5,
    }
}

fn test_book() -> Orderbook {
    Orderbook::new(test_config(), 1024, 50_000)
}

fn make_order(
    price: i64,
    qty: i64,
    side: Side,
    user_id: u32,
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::GTC,
        user_id,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1000,
        order_id_hi: 0,
        order_id_lo: user_id as u64,
    }
}

#[test]
fn new_order_valid_tick_lot_accepted() {
    let mut book = test_book();
    // tick_size=10, lot_size=5 -> price=50000, qty=10
    let mut order = make_order(50_000, 10, Side::Buy, 1);
    process_new_order(&mut book, &mut order);
    let events = book.events();
    // Should get OrderInserted (resting, no cross)
    assert!(events.iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

#[test]
fn new_order_invalid_tick_rejected() {
    let mut book = test_book();
    // tick_size=10, price=50003 not divisible
    let mut order = make_order(50_003, 10, Side::Buy, 1);
    process_new_order(&mut book, &mut order);
    let events = book.events();
    assert_eq!(events.len(), 1);
    match events[0] {
        Event::OrderFailed { reason, .. } => {
            assert_eq!(reason, FAIL_VALIDATION);
        }
        _ => panic!("expected OrderFailed"),
    }
}

#[test]
fn new_order_invalid_lot_rejected() {
    let mut book = test_book();
    // lot_size=5, qty=3 not divisible
    let mut order = make_order(50_000, 3, Side::Buy, 1);
    process_new_order(&mut book, &mut order);
    let events = book.events();
    assert_eq!(events.len(), 1);
    match events[0] {
        Event::OrderFailed { reason, .. } => {
            assert_eq!(reason, FAIL_VALIDATION);
        }
        _ => panic!("expected OrderFailed"),
    }
}

#[test]
fn new_order_zero_qty_rejected() {
    let mut book = test_book();
    let mut order = make_order(50_000, 0, Side::Buy, 1);
    process_new_order(&mut book, &mut order);
    let events = book.events();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::OrderFailed {
            reason: FAIL_VALIDATION,
            ..
        }
    ));
}

#[test]
fn new_order_negative_price_rejected() {
    let mut book = test_book();
    let mut order = make_order(-10, 10, Side::Buy, 1);
    process_new_order(&mut book, &mut order);
    let events = book.events();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::OrderFailed {
            reason: FAIL_VALIDATION,
            ..
        }
    ));
}

#[test]
fn new_order_duplicate_id_rejected() {
    let mut dedup = DedupTracker::new();
    // First insert: not duplicate
    assert!(!dedup.check_and_insert(1, 0, 42));
    // Second insert: duplicate
    assert!(dedup.check_and_insert(1, 0, 42));
}

#[test]
fn new_order_after_dedup_window_accepted() {
    let mut dedup = DedupTracker::new();
    assert!(!dedup.check_and_insert(1, 0, 42));
    assert!(dedup.check_and_insert(1, 0, 42));
    // Force cleanup with future cutoff
    dedup.cleanup_with_cutoff(
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(dedup.len(), 0);
    // Same ID now accepted again
    assert!(!dedup.check_and_insert(1, 0, 42));
}
