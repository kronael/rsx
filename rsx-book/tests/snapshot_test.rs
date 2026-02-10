use rsx_book::book::BookState;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::snapshot;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rsx_types::NONE;

fn cfg() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    }
}

fn make_book() -> Box<Orderbook> {
    Box::new(Orderbook::new(cfg(), 256, 50_000))
}

fn roundtrip(
    book: &Orderbook,
) -> Box<Orderbook> {
    let mut buf = Vec::new();
    snapshot::save(book, &mut buf).unwrap();
    let mut cursor = std::io::Cursor::new(buf);
    snapshot::load(&mut cursor).unwrap()
}

#[test]
fn empty_book_roundtrip() {
    let book = make_book();
    let restored = roundtrip(&book);
    assert_eq!(restored.sequence, 0);
    assert_eq!(restored.best_bid_tick, NONE);
    assert_eq!(restored.best_ask_tick, NONE);
    assert_eq!(restored.config.symbol_id, 1);
}

#[test]
fn single_order_roundtrip() {
    let mut book = make_book();
    book.insert_resting(
        50_000, 10, Side::Buy, 0, 42, false,
        1000, 0, 1,
    );
    let restored = roundtrip(&book);
    assert_eq!(restored.sequence, 1);
    assert_ne!(restored.best_bid_tick, NONE);
    assert_eq!(restored.best_ask_tick, NONE);
    // Check the order exists
    let slot = restored.orders.get(0);
    assert!(slot.is_active());
    assert_eq!(slot.user_id, 42);
    assert_eq!(slot.remaining_qty.0, 10);
    assert_eq!(slot.order_id_lo, 1);
}

#[test]
fn multiple_orders_both_sides() {
    let mut book = make_book();
    book.insert_resting(
        49_990, 5, Side::Buy, 0, 1, false,
        100, 0, 10,
    );
    book.insert_resting(
        49_995, 3, Side::Buy, 0, 2, false,
        200, 0, 20,
    );
    book.insert_resting(
        50_010, 7, Side::Sell, 0, 3, false,
        300, 0, 30,
    );
    let restored = roundtrip(&book);
    assert_eq!(restored.sequence, 3);
    assert_ne!(restored.best_bid_tick, NONE);
    assert_ne!(restored.best_ask_tick, NONE);
    assert_eq!(
        restored.best_bid_tick,
        book.best_bid_tick,
    );
    assert_eq!(
        restored.best_ask_tick,
        book.best_ask_tick,
    );
}

#[test]
fn user_state_preserved() {
    let mut book = make_book();
    book.insert_resting(
        49_990, 5, Side::Buy, 0, 42, false,
        100, 0, 1,
    );
    book.insert_resting(
        50_010, 3, Side::Sell, 0, 42, false,
        200, 0, 2,
    );
    let restored = roundtrip(&book);
    let idx = restored.user_map.get(&42).unwrap();
    let us =
        &restored.user_states[*idx as usize];
    assert_eq!(us.user_id, 42);
    assert_eq!(us.order_count, 2);
}

#[test]
fn sequence_preserved() {
    let mut book = make_book();
    for i in 0..10 {
        book.insert_resting(
            49_990 + i, 1, Side::Buy, 0,
            1, false, 100, 0, i as u64,
        );
    }
    assert_eq!(book.sequence, 10);
    let restored = roundtrip(&book);
    assert_eq!(restored.sequence, 10);
}

#[test]
fn cancel_then_snapshot_preserves_state() {
    let mut book = make_book();
    let h1 = book.insert_resting(
        49_990, 5, Side::Buy, 0, 1, false,
        100, 0, 1,
    );
    let _h2 = book.insert_resting(
        49_995, 3, Side::Buy, 0, 2, false,
        200, 0, 2,
    );
    book.cancel_order(h1);
    let restored = roundtrip(&book);
    // Only one active order
    let slot0 = restored.orders.get(0);
    assert!(!slot0.is_active());
    let slot1 = restored.orders.get(1);
    assert!(slot1.is_active());
    assert_eq!(slot1.user_id, 2);
}

#[test]
fn snapshot_during_migration_fails() {
    let mut book = make_book();
    book.state = BookState::Migrating;
    let mut buf = Vec::new();
    let result = snapshot::save(&book, &mut buf);
    assert!(result.is_err());
}

#[test]
fn fill_then_snapshot_user_positions() {
    let mut book = make_book();
    // Insert a resting sell
    book.insert_resting(
        50_000, 10, Side::Sell, 0, 2, false,
        100, 0, 20,
    );
    // Match with aggressive buy
    let mut incoming = IncomingOrder {
        price: 50_000,
        qty: 5,
        remaining_qty: 5,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 200,
        order_id_hi: 0,
        order_id_lo: 10,
    };
    book.event_len = 0;
    process_new_order(&mut book, &mut incoming);

    let restored = roundtrip(&book);

    // User 1 (taker buy): +5 position
    let idx1 =
        restored.user_map.get(&1).unwrap();
    assert_eq!(
        restored.user_states[*idx1 as usize]
            .net_qty,
        5,
    );
    // User 2 (maker sell): -5 position
    let idx2 =
        restored.user_map.get(&2).unwrap();
    assert_eq!(
        restored.user_states[*idx2 as usize]
            .net_qty,
        -5,
    );
}

#[test]
fn invalid_magic_rejected() {
    let data = vec![0u8; 100];
    let mut cursor = std::io::Cursor::new(data);
    let result = snapshot::load(&mut cursor);
    assert!(result.is_err());
}
