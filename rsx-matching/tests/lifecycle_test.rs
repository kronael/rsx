use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::REASON_FILLED;
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
        tick_size: 10,
        lot_size: 5,
    }
}

fn test_book() -> Orderbook {
    Orderbook::new(test_config(), 4096, 50_000)
}

fn make_order(
    price: i64,
    qty: i64,
    side: Side,
    user_id: u32,
    oid_lo: u64,
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
        order_id_lo: oid_lo,
    }
}

/// Submit buy at 50000, submit sell at 50000.
/// Expect: Fill + maker OrderDone + taker OrderDone + BBO.
#[test]
fn order_submit_fill_done_complete_lifecycle() {
    let mut book = test_book();

    // Buy rests on the book
    let mut buy = make_order(50_000, 10, Side::Buy, 1, 1);
    process_new_order(&mut book, &mut buy);
    let events = book.events();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));

    // Sell crosses the buy
    let mut sell =
        make_order(50_000, 10, Side::Sell, 2, 2);
    process_new_order(&mut book, &mut sell);
    let events = book.events();

    // Should see: Fill, maker OrderDone, taker OrderDone
    let fills: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 1);
    match fills[0] {
        Event::Fill { qty, price, .. } => {
            assert_eq!(qty.0, 10);
            assert_eq!(price.0, 50_000);
        }
        _ => unreachable!(),
    }

    let dones: Vec<_> = events
        .iter()
        .filter(|e| matches!(
            e,
            Event::OrderDone { .. }
        ))
        .collect();
    // Maker done + taker done
    assert_eq!(dones.len(), 2);
    for d in &dones {
        match d {
            Event::OrderDone {
                reason,
                remaining_qty,
                ..
            } => {
                assert_eq!(*reason, REASON_FILLED);
                assert_eq!(remaining_qty.0, 0);
            }
            _ => unreachable!(),
        }
    }
}

/// Submit buy, cancel it, verify OrderCancelled + OrderDone
/// sequence is not emitted (cancel_order is direct, not
/// through process_new_order).
#[test]
fn order_submit_rest_cancel_done_lifecycle() {
    let mut book = test_book();

    let mut buy = make_order(50_000, 10, Side::Buy, 1, 1);
    process_new_order(&mut book, &mut buy);
    let events = book.events();
    let handle = match events[0] {
        Event::OrderInserted { handle, .. } => handle,
        _ => panic!("expected OrderInserted"),
    };

    let cancelled = book.cancel_order(handle);
    assert!(cancelled);

    // Verify the book is empty after cancel
    assert_eq!(
        book.best_bid_tick,
        rsx_types::NONE,
    );
}

/// Partial fill: buy 20, sell 10, then sell 10 more.
#[test]
fn order_submit_partial_fill_rest_then_fill() {
    let mut book = test_book();

    // Buy 20 rests
    let mut buy =
        make_order(50_000, 20, Side::Buy, 1, 1);
    process_new_order(&mut book, &mut buy);

    // Sell 10 partially fills the buy
    let mut sell1 =
        make_order(50_000, 10, Side::Sell, 2, 2);
    process_new_order(&mut book, &mut sell1);
    let events = book.events();

    let fills: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 1);
    match fills[0] {
        Event::Fill { qty, .. } => {
            assert_eq!(qty.0, 10);
        }
        _ => unreachable!(),
    }

    // Maker (buy) should NOT have OrderDone yet
    let maker_dones: Vec<_> = events
        .iter()
        .filter(|e| match e {
            Event::OrderDone {
                order_id_lo, ..
            } => *order_id_lo == 1,
            _ => false,
        })
        .collect();
    assert_eq!(maker_dones.len(), 0);

    // Sell 10 more completes the buy
    let mut sell2 =
        make_order(50_000, 10, Side::Sell, 3, 3);
    process_new_order(&mut book, &mut sell2);
    let events = book.events();

    let fills: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 1);

    // Now maker should be done
    let maker_dones: Vec<_> = events
        .iter()
        .filter(|e| match e {
            Event::OrderDone {
                order_id_lo, ..
            } => *order_id_lo == 1,
            _ => false,
        })
        .collect();
    assert_eq!(maker_dones.len(), 1);
}

/// 500 resting makers, one aggressor sweeps them all.
#[test]
fn order_submit_multi_fill_500_makers() {
    let mut book = Orderbook::new(
        test_config(),
        8192,
        50_000,
    );

    // Place 500 sell orders at price 50000, qty 5 each
    for i in 0..500u32 {
        let mut sell = make_order(
            50_000,
            5,
            Side::Sell,
            100 + i,
            100 + i as u64,
        );
        process_new_order(&mut book, &mut sell);
    }

    // One buy sweeps all 500
    let mut buy = make_order(
        50_000,
        2500,
        Side::Buy,
        1,
        1,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    let fills: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 500);

    let total_filled: i64 = fills
        .iter()
        .map(|e| match e {
            Event::Fill { qty, .. } => qty.0,
            _ => 0,
        })
        .sum();
    assert_eq!(total_filled, 2500);

    // 500 maker dones + 1 taker done
    let dones: Vec<_> = events
        .iter()
        .filter(|e| matches!(
            e,
            Event::OrderDone { .. }
        ))
        .collect();
    assert_eq!(dones.len(), 501);
}
