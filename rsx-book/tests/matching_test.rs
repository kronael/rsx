use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::FAIL_FOK;
use rsx_book::event::FAIL_REDUCE_ONLY;
use rsx_book::event::FAIL_VALIDATION;
use rsx_book::event::REASON_CANCELLED;
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
    tif: TimeInForce,
    user_id: u32,
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif,
        user_id,
        reduce_only: false,
        timestamp_ns: 0,
    }
}

#[test]
fn match_buy_against_single_ask() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let events = book.events();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Fill { qty, .. }
            if qty.0 == 100
    )));
}

#[test]
fn match_sell_against_single_bid() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0,
    );
    let mut order =
        incoming(49_900, 100, Side::Sell, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::Fill { qty, .. }
            if qty.0 == 100
    )));
}

#[test]
fn match_multiple_makers_same_level() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 2, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 2);
}

#[test]
fn match_crosses_multiple_levels() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    book.insert_resting(
        50_101, 50, Side::Sell, 0, 2, false, 0,
    );
    let mut order =
        incoming(50_101, 100, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 2);
}

#[test]
fn match_partial_fill_maker_remains() {
    let mut book = test_book();
    let h = book.insert_resting(
        50_100, 200, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // Maker still in book with 100 remaining
    assert!(book.orders.get(h).is_active());
    assert_eq!(book.orders.get(h).remaining_qty, 100);
}

#[test]
fn match_partial_fill_taker_rests() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // Taker should have been inserted as resting
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { qty, .. }
            if qty.0 == 50
    )));
}

#[test]
fn match_no_cross_taker_rests() {
    let mut book = test_book();
    book.insert_resting(
        50_200, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 0);
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

#[test]
fn match_fill_price_is_maker_price() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    // Taker buys at 50_200, but fill should be at
    // maker's 50_100
    let mut order =
        incoming(50_200, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::Fill { price, .. }
            if price.0 == 50_100
    )));
}

#[test]
fn match_fifo_within_level() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let _h2 = book.insert_resting(
        50_100, 100, Side::Sell, 0, 2, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    // First fill should be against h1 (first maker)
    if let Event::Fill { maker_handle, .. } =
        book.events()[0]
    {
        assert_eq!(maker_handle, h1);
    } else {
        panic!("expected fill");
    }
}

#[test]
fn ioc_cancels_remainder() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::IOC, 2);
    process_new_order(&mut book, &mut order);

    // Should have fill + OrderDone(cancelled)
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderDone {
            reason, ..
        } if *reason == REASON_CANCELLED
    )));
    // No OrderInserted
    assert!(!book.events().iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

#[test]
fn fok_rejects_if_not_full() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::FOK, 2);
    process_new_order(&mut book, &mut order);

    // Should have OrderFailed(FOK)
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == FAIL_FOK
    )));
    // No fills in final events (rolled back)
    assert!(!book.events().iter().any(|e| matches!(
        e,
        Event::Fill { .. }
    )));
}

#[test]
fn fok_succeeds_when_full() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::FOK, 2);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::Fill { .. }
    )));
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderDone {
            reason, ..
        } if *reason == REASON_FILLED
    )));
}

#[test]
fn fills_precede_order_done() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let events = book.events();
    let fill_pos = events
        .iter()
        .position(|e| matches!(e, Event::Fill { .. }))
        .unwrap();
    let done_pos = events
        .iter()
        .rposition(|e| {
            matches!(e, Event::OrderDone { .. })
        })
        .unwrap();
    assert!(fill_pos < done_pos);
}

#[test]
fn exactly_one_completion() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // Taker gets exactly one OrderDone
    let taker_dones: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(
            e,
            Event::OrderDone { user_id, .. }
                if *user_id == 2
        ))
        .collect();
    assert_eq!(taker_dones.len(), 1);
}

#[test]
fn reduce_only_rejected_if_no_position() {
    let mut book = test_book();
    let mut order =
        incoming(49_900, 100, Side::Buy, TimeInForce::GTC, 1);
    order.reduce_only = true;
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == FAIL_REDUCE_ONLY
    )));
}

#[test]
fn reduce_only_buy_rejected_if_long() {
    let mut book = test_book();
    // Create a long position: buy fills
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 2, false, 0,
    );
    let mut buy =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut buy);

    // Now try reduce-only buy (user is long, so
    // buying more is rejected)
    let mut ro =
        incoming(49_900, 50, Side::Buy, TimeInForce::GTC, 1);
    ro.reduce_only = true;
    process_new_order(&mut book, &mut ro);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == FAIL_REDUCE_ONLY
    )));
}

#[test]
fn reduce_only_sell_accepted_if_long() {
    let mut book = test_book();
    // Create a long position
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 2, false, 0,
    );
    let mut buy =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut buy);

    // Reduce-only sell should be accepted (closing)
    book.insert_resting(
        50_050, 100, Side::Buy, 0, 3, false, 0,
    );
    let mut ro =
        incoming(50_050, 50, Side::Sell, TimeInForce::GTC, 1);
    ro.reduce_only = true;
    process_new_order(&mut book, &mut ro);

    // Should NOT be OrderFailed
    assert!(!book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed { .. }
    )));
}

#[test]
fn position_tracking_on_fills() {
    let mut book = test_book();
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0,
    );
    let mut order =
        incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // User 2 bought 100 -> net_qty = +100
    let u2_idx = book.user_map.get(&2).unwrap();
    assert_eq!(
        book.user_states[*u2_idx as usize].net_qty,
        100
    );
    // User 1 sold 100 -> net_qty = -100
    let u1_idx = book.user_map.get(&1).unwrap();
    assert_eq!(
        book.user_states[*u1_idx as usize].net_qty,
        -100
    );
}

#[test]
fn event_buffer_reset_each_cycle() {
    let mut book = test_book();
    let mut o1 =
        incoming(49_900, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut o1);
    let _len1 = book.event_len;

    let mut o2 =
        incoming(49_800, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut o2);

    // event_len should have been reset at start of
    // process_new_order
    assert_eq!(book.event_len, 1); // just OrderInserted
}

#[test]
fn validation_failure_emits_order_failed() {
    let mut book = test_book();
    // qty=0 should fail validation
    let mut order =
        incoming(50_100, 0, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == FAIL_VALIDATION
    )));
}

// ── Smooshed-tick tests ─────────────────────────

/// Two orders at different raw prices map to same tick
/// (compression > 1). Both should coexist in the level.
#[test]
fn smooshed_level_orders_with_different_prices() {
    // Use config where tick_size=1, mid=50_000.
    // Zone 1 has compression=10, so prices 52_500 and
    // 52_509 map to the same slot.
    let mut book = test_book();
    let h1 = book.insert_resting(
        52_500, 100, Side::Sell, 0, 1, false, 0,
    );
    let h2 = book.insert_resting(
        52_509, 100, Side::Sell, 0, 2, false, 0,
    );
    let t1 = book.orders.get(h1).tick_index;
    let t2 = book.orders.get(h2).tick_index;
    assert_eq!(t1, t2); // same tick slot
    let level = &book.active_levels[t1 as usize];
    assert_eq!(level.order_count, 2);
}

/// Buy aggressor at 52_500 should skip a resting sell
/// at 52_509 (within same smooshed tick) because maker
/// price > limit.
#[test]
fn smooshed_match_skips_orders_outside_limit() {
    let mut book = test_book();
    // Resting sell at 52_509 in zone 1 (compression=10)
    book.insert_resting(
        52_509, 100, Side::Sell, 0, 1, false, 0,
    );
    // Buy at 52_500 -- can't afford 52_509
    let mut order = incoming(
        52_500, 100, Side::Buy, TimeInForce::GTC, 2,
    );
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 0);
}

/// Buy at 52_509 should fill the resting sell at
/// 52_500 (maker price <= limit).
#[test]
fn smooshed_match_fills_qualifying_orders_only() {
    let mut book = test_book();
    book.insert_resting(
        52_500, 50, Side::Sell, 0, 1, false, 0,
    );
    book.insert_resting(
        52_509, 50, Side::Sell, 0, 2, false, 0,
    );
    // Buy at 52_505: fills 52_500 maker, skips 52_509
    let mut order = incoming(
        52_505, 100, Side::Buy, TimeInForce::GTC, 3,
    );
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 1);
}

/// Within a smooshed tick, FIFO order is preserved.
#[test]
fn smooshed_match_preserves_time_priority() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        52_500, 50, Side::Sell, 0, 1, false, 0,
    );
    let _h2 = book.insert_resting(
        52_501, 50, Side::Sell, 0, 2, false, 0,
    );
    // Both at same tick. Buy enough for one fill.
    let mut order = incoming(
        52_501, 50, Side::Buy, TimeInForce::GTC, 3,
    );
    process_new_order(&mut book, &mut order);

    // First fill should be h1 (earlier insert)
    if let Event::Fill { maker_handle, .. } =
        book.events()[0]
    {
        assert_eq!(maker_handle, h1);
    } else {
        panic!("expected fill");
    }
}

// ── Event buffer tests ──────────────────────────

/// Multiple partial fills against one maker produce
/// multiple Fill events.
#[test]
fn event_buffer_multiple_fills_single_order() {
    let mut book = test_book();
    // Two sells at different levels
    book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    book.insert_resting(
        50_101, 50, Side::Sell, 0, 2, false, 0,
    );
    let mut order = incoming(
        50_101, 100, Side::Buy, TimeInForce::GTC, 3,
    );
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 2);
}

/// For each maker, its Fill event precedes its OrderDone.
#[test]
fn event_buffer_fills_before_done() {
    let mut book = test_book();
    let h1 = book.insert_resting(
        50_100, 50, Side::Sell, 0, 1, false, 0,
    );
    let h2 = book.insert_resting(
        50_101, 50, Side::Sell, 0, 2, false, 0,
    );
    let mut order = incoming(
        50_101, 100, Side::Buy, TimeInForce::GTC, 3,
    );
    process_new_order(&mut book, &mut order);

    let events = book.events();
    // For each maker handle, fill comes before done
    for handle in [h1, h2] {
        let fill_pos = events
            .iter()
            .position(|e| matches!(
                e,
                Event::Fill { maker_handle, .. }
                    if *maker_handle == handle
            ))
            .unwrap();
        let done_pos = events
            .iter()
            .position(|e| matches!(
                e,
                Event::OrderDone { handle: h, .. }
                    if *h == handle
            ))
            .unwrap();
        assert!(
            fill_pos < done_pos,
            "fill at {} should precede done at {} for handle {}",
            fill_pos, done_pos, handle,
        );
    }
}
