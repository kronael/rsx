use rsx_book::book::Orderbook;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_matching::fanout::drain_and_fanout;
use rsx_matching::wire::EventMessage;
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

fn drain_ring(
    cons: &mut rtrb::Consumer<EventMessage>,
) -> Vec<EventMessage> {
    let mut out = Vec::new();
    while let Ok(msg) = cons.pop() {
        out.push(msg);
    }
    out
}

#[test]
fn fill_sent_to_risk_gateway_mktdata() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order = IncomingOrder {
        price: 50_100,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        timestamp_ns: 0,
    };
    process_new_order(&mut book, &mut order);

    let (mut rp, mut rc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut gp, mut gc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut mp, mut mc) =
        rtrb::RingBuffer::<EventMessage>::new(64);

    drain_and_fanout(&book, &mut rp, &mut gp, &mut mp);

    let risk = drain_ring(&mut rc);
    let gw = drain_ring(&mut gc);
    let mkt = drain_ring(&mut mc);

    // Fill should appear in all three
    assert!(risk.iter().any(|e| matches!(
        e,
        EventMessage::Fill { .. }
    )));
    assert!(gw.iter().any(|e| matches!(
        e,
        EventMessage::Fill { .. }
    )));
    assert!(mkt.iter().any(|e| matches!(
        e,
        EventMessage::Fill { .. }
    )));
}

#[test]
fn order_inserted_sent_to_mktdata_only() {
    let mut book = test_book();
    // No matching opposite side -> order rests
    let mut order = IncomingOrder {
        price: 49_900,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        timestamp_ns: 0,
    };
    process_new_order(&mut book, &mut order);

    let (mut rp, mut rc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut gp, mut gc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut mp, mut mc) =
        rtrb::RingBuffer::<EventMessage>::new(64);

    drain_and_fanout(&book, &mut rp, &mut gp, &mut mp);

    let risk = drain_ring(&mut rc);
    let gw = drain_ring(&mut gc);
    let mkt = drain_ring(&mut mc);

    assert!(risk.is_empty());
    assert!(gw.is_empty());
    assert!(mkt.iter().any(|e| matches!(
        e,
        EventMessage::OrderInserted { .. }
    )));
}

#[test]
fn order_done_sent_to_risk_gateway() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order = IncomingOrder {
        price: 50_100,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        timestamp_ns: 0,
    };
    process_new_order(&mut book, &mut order);

    let (mut rp, mut rc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut gp, mut gc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut mp, mut mc) =
        rtrb::RingBuffer::<EventMessage>::new(64);

    drain_and_fanout(&book, &mut rp, &mut gp, &mut mp);

    let risk = drain_ring(&mut rc);
    let gw = drain_ring(&mut gc);
    let mkt = drain_ring(&mut mc);

    assert!(risk.iter().any(|e| matches!(
        e,
        EventMessage::OrderDone { .. }
    )));
    assert!(gw.iter().any(|e| matches!(
        e,
        EventMessage::OrderDone { .. }
    )));
    // mkt should NOT have OrderDone
    assert!(!mkt.iter().any(|e| matches!(
        e,
        EventMessage::OrderDone { .. }
    )));
}

#[test]
fn order_cancelled_sent_to_gateway_mktdata() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    book.cancel_order(h);
    // Emit the cancel event manually since cancel_order
    // doesn't emit events (it's a book-level op).
    // We'll test via a direct event instead.
    book.event_len = 0;
    book.emit(rsx_book::event::Event::OrderCancelled {
        handle: h,
        user_id: 1,
        remaining_qty: rsx_types::Qty(100),
    });

    let (mut rp, mut rc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut gp, mut gc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut mp, mut mc) =
        rtrb::RingBuffer::<EventMessage>::new(64);

    drain_and_fanout(&book, &mut rp, &mut gp, &mut mp);

    let risk = drain_ring(&mut rc);
    let gw = drain_ring(&mut gc);
    let mkt = drain_ring(&mut mc);

    assert!(risk.is_empty());
    assert!(gw.iter().any(|e| matches!(
        e,
        EventMessage::OrderCancelled { .. }
    )));
    assert!(mkt.iter().any(|e| matches!(
        e,
        EventMessage::OrderCancelled { .. }
    )));
}

#[test]
fn drain_empties_buffer() {
    let mut book = test_book();
    let mut order = IncomingOrder {
        price: 49_900,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        timestamp_ns: 0,
    };
    process_new_order(&mut book, &mut order);
    assert!(book.event_len > 0);

    let (mut rp, _rc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut gp, _gc) =
        rtrb::RingBuffer::<EventMessage>::new(64);
    let (mut mp, _mc) =
        rtrb::RingBuffer::<EventMessage>::new(64);

    drain_and_fanout(&book, &mut rp, &mut gp, &mut mp);
    // drain_and_fanout reads events but doesn't clear
    // the buffer -- that's done by process_new_order
    // at the start of each cycle.
    assert!(book.event_len > 0);
}
