use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderDoneReason;
use rsx_book::event::OrderFailureReason;
use rsx_book::event::MAX_EVENTS;
use rsx_book::event::MAX_MAKERS_PER_ORDER;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rsx_types::NONE;

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

fn incoming(price: i64, qty: i64, side: Side, tif: TimeInForce, user_id: u32) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif,
        user_id,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 0,
    }
}

// An empty-book reduce-only IOC with qty > position clamps remaining to
// the position, then cancels with ZERO fills — the clamp must not be
// reported as executed quantity (ME-REDUCEONLY-IOC-FILLEDQTY).
#[test]
fn reduce_only_ioc_empty_book_reports_zero_filled() {
    let mut book = test_book();
    // Give user 7 a long position of 30 via a fill (counterparty 999),
    // no resting book to hit.
    book.users.apply_fill(7, 999, Side::Buy, 30);

    let mut order = IncomingOrder {
        price: 49_900,
        qty: 100,
        remaining_qty: 100,
        side: Side::Sell,
        tif: TimeInForce::IOC,
        user_id: 7,
        reduce_only: true,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut order);

    assert!(
        !book
            .events()
            .iter()
            .any(|e| matches!(e, Event::Fill { .. })),
        "empty-book reduce-only IOC must not fill",
    );
    let done = book.events().iter().find_map(|e| match e {
        Event::OrderDone {
            reason,
            filled_qty,
            remaining_qty,
            ..
        } => Some((*reason, filled_qty.0, remaining_qty.0)),
        _ => None,
    });
    assert_eq!(
        done,
        Some((OrderDoneReason::Expired, 0, 30)),
        "clamp must not count as fill: filled=0, remaining=30",
    );
}

#[test]
fn match_buy_against_single_ask() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
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
    book.insert_resting(49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0);
    let mut order = incoming(49_900, 100, Side::Sell, TimeInForce::GTC, 2);
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
    book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(50_100, 50, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 3);
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
    book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(50_101, 50, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut order = incoming(50_101, 100, Side::Buy, TimeInForce::GTC, 3);
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
    let h = book.insert_resting(50_100, 200, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // Maker still in book with 100 remaining
    assert!(book.orders.get(h).is_active());
    assert_eq!(book.orders.get(h).remaining_qty, Qty(100));
}

#[test]
fn match_partial_fill_taker_rests() {
    let mut book = test_book();
    book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
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
    book.insert_resting(50_200, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 0);
    assert!(book
        .events()
        .iter()
        .any(|e| matches!(e, Event::OrderInserted { .. })));
}

#[test]
fn match_fill_price_is_maker_price() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    // Taker buys at 50_200, but fill should be at
    // maker's 50_100
    let mut order = incoming(50_200, 100, Side::Buy, TimeInForce::GTC, 2);
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
    let h1 = book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let _h2 = book.insert_resting(50_100, 100, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    // First fill should be against h1 (first maker)
    if let Event::Fill { maker_handle, .. } = book.events()[0] {
        assert_eq!(maker_handle, h1);
    } else {
        panic!("expected fill");
    }
}

#[test]
#[ignore = "release-only MAX_EVENTS boundary evidence; covered by deep_million"]
#[should_panic(expected = "INVARIANT: ME event buffer overflow (MAX_EVENTS=65536)")]
fn oversize_aggressor_panics_instead_of_truncating_events() {
    let fills = rsx_book::event::MAX_EVENTS / 2;
    let mut book = Orderbook::new(test_config(), fills as u32, 50_000);
    for oid in 0..fills as u64 {
        book.insert_resting(50_100, 1, Side::Sell, 0, 1, false, oid, 0, oid);
    }
    let mut order = incoming(50_100, fills as i64, Side::Buy, TimeInForce::IOC, 2);
    process_new_order(&mut book, &mut order);
}

#[test]
fn ioc_cancels_remainder() {
    let mut book = test_book();
    book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::IOC, 2);
    process_new_order(&mut book, &mut order);

    // Should have fill + OrderDone(expired)
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderDone {
            reason, ..
        } if *reason == OrderDoneReason::Expired
    )));
    // No OrderInserted
    assert!(!book
        .events()
        .iter()
        .any(|e| matches!(e, Event::OrderInserted { .. })));
}

#[test]
fn fok_rejects_if_not_full() {
    let mut book = test_book();
    book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::FOK, 2);
    process_new_order(&mut book, &mut order);

    // Should have OrderFailed(FOK)
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == OrderFailureReason::FillOrKill
    )));
    // No fills in final events (rolled back)
    assert!(!book
        .events()
        .iter()
        .any(|e| matches!(e, Event::Fill { .. })));
}

#[test]
fn fok_succeeds_when_full() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::FOK, 2);
    process_new_order(&mut book, &mut order);

    assert!(book
        .events()
        .iter()
        .any(|e| matches!(e, Event::Fill { .. })));
    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderDone {
            reason, ..
        } if *reason == OrderDoneReason::Filled
    )));
}

#[test]
fn fills_precede_order_done() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let events = book.events();
    let fill_pos = events
        .iter()
        .position(|e| matches!(e, Event::Fill { .. }))
        .unwrap();
    let done_pos = events
        .iter()
        .rposition(|e| matches!(e, Event::OrderDone { .. }))
        .unwrap();
    assert!(fill_pos < done_pos);
}

#[test]
fn exactly_one_completion() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // Taker gets exactly one OrderDone
    let taker_dones: Vec<_> = book
        .events()
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::OrderDone { user_id, .. }
                    if *user_id == 2
            )
        })
        .collect();
    assert_eq!(taker_dones.len(), 1);
}

#[test]
fn reduce_only_rejected_if_no_position() {
    let mut book = test_book();
    let mut order = incoming(49_900, 100, Side::Buy, TimeInForce::GTC, 1);
    order.reduce_only = true;
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == OrderFailureReason::ReduceOnly
    )));
}

#[test]
fn reduce_only_buy_rejected_if_long() {
    let mut book = test_book();
    // Create a long position: buy fills
    book.insert_resting(50_100, 100, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut buy = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut buy);

    // Now try reduce-only buy (user is long, so
    // buying more is rejected)
    let mut ro = incoming(49_900, 50, Side::Buy, TimeInForce::GTC, 1);
    ro.reduce_only = true;
    process_new_order(&mut book, &mut ro);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == OrderFailureReason::ReduceOnly
    )));
}

#[test]
fn reduce_only_sell_accepted_if_long() {
    let mut book = test_book();
    // Create a long position
    book.insert_resting(50_100, 100, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut buy = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut buy);

    // Reduce-only sell should be accepted (closing)
    book.insert_resting(50_050, 100, Side::Buy, 0, 3, false, 0, 0, 0);
    let mut ro = incoming(50_050, 50, Side::Sell, TimeInForce::GTC, 1);
    ro.reduce_only = true;
    process_new_order(&mut book, &mut ro);

    // Should NOT be OrderFailed
    assert!(!book
        .events()
        .iter()
        .any(|e| matches!(e, Event::OrderFailed { .. })));
}

#[test]
fn position_tracking_on_fills() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    // User 2 bought 100 -> net_qty = +100
    assert_eq!(book.users.net_qty(2), Some(100));
    // User 1 sold 100 -> net_qty = -100
    assert_eq!(book.users.net_qty(1), Some(-100));
}

#[test]
fn event_buffer_reset_each_cycle() {
    let mut book = test_book();
    let mut first_order = incoming(49_900, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut first_order);

    let mut second_order = incoming(49_800, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut second_order);

    assert_eq!(book.events().len(), 1);
}

#[test]
fn validation_failure_emits_order_failed() {
    let mut book = test_book();
    // qty=0 should fail validation
    let mut order = incoming(50_100, 0, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == OrderFailureReason::InvalidLot
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
    let h1 = book.insert_resting(52_500, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    let h2 = book.insert_resting(52_509, 100, Side::Sell, 0, 2, false, 0, 0, 0);
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
    book.insert_resting(52_509, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    // Buy at 52_500 -- can't afford 52_509
    let mut order = incoming(52_500, 100, Side::Buy, TimeInForce::GTC, 2);
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
    book.insert_resting(52_500, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(52_509, 50, Side::Sell, 0, 2, false, 0, 0, 0);
    // Buy at 52_505: fills 52_500 maker, skips 52_509
    let mut order = incoming(52_505, 100, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    let fills: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(fills.len(), 1);
}

/// Within a smooshed tick, makers at one price fill in arrival order.
#[test]
fn smooshed_match_preserves_time_priority() {
    let mut book = test_book();
    let h1 = book.insert_resting(52_501, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    let _h2 = book.insert_resting(52_501, 50, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut order = incoming(52_501, 50, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    if let Event::Fill { maker_handle, .. } = book.events()[0] {
        assert_eq!(maker_handle, h1);
    } else {
        panic!("expected fill");
    }
}

fn fills_in_order(book: &Orderbook) -> Vec<(u32, i64, i64)> {
    book.events()
        .iter()
        .filter_map(|e| match e {
            Event::Fill {
                maker_user_id,
                price,
                qty,
                ..
            } => Some((*maker_user_id, price.0, qty.0)),
            _ => None,
        })
        .collect()
}

/// A worse price resting ahead of a better one in a compressed slot: the
/// taker fills the better price first.
#[test]
fn compressed_slot_fills_the_better_price_before_an_earlier_worse_one() {
    let mut book = test_book();
    book.insert_resting(52_505, 1, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(52_501, 1, Side::Sell, 0, 2, false, 0, 0, 1);
    assert_eq!(
        book.compression.price_to_index(52_505),
        book.compression.price_to_index(52_501),
    );
    let mut order = incoming(52_506, 1, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    assert_eq!(fills_in_order(&book), [(2, 52_501, 1)]);
    assert_eq!(book.current_bbo().ask_px, Price(52_505));
}

#[test]
fn compressed_slot_sell_taker_fills_the_higher_bid_first() {
    let mut book = test_book();
    book.insert_resting(47_495, 1, Side::Buy, 0, 1, false, 0, 0, 0);
    book.insert_resting(47_499, 1, Side::Buy, 0, 2, false, 0, 0, 1);
    assert_eq!(
        book.compression.price_to_index(47_495),
        book.compression.price_to_index(47_499),
    );
    let mut order = incoming(47_490, 1, Side::Sell, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    assert_eq!(fills_in_order(&book), [(2, 47_499, 1)]);
    assert_eq!(book.current_bbo().bid_px, Price(47_495));
}

/// A taker that covers every crossing maker in the slot fills them all.
#[test]
fn compressed_slot_taker_covering_the_slot_fills_every_maker() {
    let mut book = test_book();
    book.insert_resting(52_505, 1, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(52_501, 1, Side::Sell, 0, 2, false, 0, 0, 1);
    let mut order = incoming(52_506, 2, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    let mut fills = fills_in_order(&book);
    fills.sort();
    assert_eq!(fills, [(1, 52_505, 1), (2, 52_501, 1)]);
    assert_eq!(book.best_ask_tick, NONE);
    assert_eq!(taker_done(&book), [(OrderDoneReason::Filled, 2, 0)]);
}

/// Best price first, arrival order within it, and a partial fill of the
/// worse price once the better one is gone.
#[test]
fn compressed_slot_fills_price_then_arrival_order() {
    let mut book = test_book();
    book.insert_resting(52_503, 5, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(52_501, 2, Side::Sell, 0, 2, false, 0, 0, 1);
    book.insert_resting(52_501, 2, Side::Sell, 0, 3, false, 0, 0, 2);
    book.insert_resting(52_502, 2, Side::Sell, 0, 4, false, 0, 0, 3);
    let mut order = incoming(52_509, 5, Side::Buy, TimeInForce::GTC, 5);
    process_new_order(&mut book, &mut order);

    assert_eq!(
        fills_in_order(&book),
        [(2, 52_501, 2), (3, 52_501, 2), (4, 52_502, 1)],
    );
    assert_eq!(book.current_bbo().ask_px, Price(52_502));
    assert_eq!(book.current_bbo().ask_qty, Qty(1));
}

/// The maker cap stops a compressed slot at the best prices, not at the
/// first arrivals.
#[test]
fn compressed_slot_maker_cap_keeps_the_worst_prices_resting() {
    let mut book = Orderbook::new(test_config(), (CAP + EXTRA) as u32, 50_000);
    for i in 0..CAP + EXTRA {
        let price = 52_500 + i as i64 % 10;
        book.insert_resting(price, 1, Side::Sell, 0, 1, false, 0, 0, i as u64);
    }
    let mut order = incoming(52_509, (CAP + EXTRA) as i64, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let filled = fills_in_order(&book);
    assert_eq!(filled.len(), CAP);
    assert_eq!(
        taker_done(&book),
        [(OrderDoneReason::MatchLimit, CAP as i64, EXTRA as i64)],
    );
    let worst_filled = filled.iter().map(|&(_, price, _)| price).max().unwrap();
    assert!(worst_filled <= book.current_bbo().ask_px.0);
    assert_eq!(book.current_bbo().ask_qty, Qty(EXTRA as i64));
}

// ── Event buffer tests ──────────────────────────

/// Multiple partial fills against one maker produce
/// multiple Fill events.
#[test]
fn event_buffer_multiple_fills_single_order() {
    let mut book = test_book();
    // Two sells at different levels
    book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    book.insert_resting(50_101, 50, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut order = incoming(50_101, 100, Side::Buy, TimeInForce::GTC, 3);
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
    let h1 = book.insert_resting(50_100, 50, Side::Sell, 0, 1, false, 0, 0, 0);
    let h2 = book.insert_resting(50_101, 50, Side::Sell, 0, 2, false, 0, 0, 0);
    let mut order = incoming(50_101, 100, Side::Buy, TimeInForce::GTC, 3);
    process_new_order(&mut book, &mut order);

    let events = book.events();
    // For each maker handle, fill comes before done
    for handle in [h1, h2] {
        let fill_pos = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    Event::Fill { maker_handle, .. }
                        if *maker_handle == handle
                )
            })
            .unwrap();
        let done_pos = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    Event::OrderDone { handle: h, .. }
                        if *h == handle
                )
            })
            .unwrap();
        assert!(
            fill_pos < done_pos,
            "fill at {} should precede done at {} for handle {}",
            fill_pos,
            done_pos,
            handle,
        );
    }
}

// ── BBO emission tests ─────────────────────────

#[test]
fn bbo_emitted_after_new_best_bid() {
    let mut book = test_book();
    let mut order = incoming(49_900, 100, Side::Buy, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut order);

    let bbo: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::BBO { .. }))
        .collect();
    assert_eq!(bbo.len(), 1);
    if let Event::BBO {
        bid_px,
        bid_qty,
        ask_px,
        ask_qty,
    } = bbo[0]
    {
        assert_eq!(*bid_px, Price(49_900));
        assert_eq!(*bid_qty, Qty(100));
        assert_eq!(*ask_px, Price(0));
        assert_eq!(*ask_qty, Qty(0));
    }
}

#[test]
fn bbo_emitted_after_fill_removes_best() {
    let mut book = test_book();
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
    // Buy sweeps the only ask level
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let bbo: Vec<_> = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::BBO { .. }))
        .collect();
    assert_eq!(bbo.len(), 1);
    // Ask should now be empty
    if let Event::BBO {
        ask_px, ask_qty, ..
    } = bbo[0]
    {
        assert_eq!(*ask_px, Price(0));
        assert_eq!(*ask_qty, Qty(0));
    }
}

#[test]
fn bbo_not_emitted_when_best_unchanged() {
    let mut book = test_book();
    // Insert best bid at 49_900
    book.insert_resting(49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0);
    // Insert worse bid at 49_800 -- best unchanged
    let mut order = incoming(49_800, 50, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    let bbo_count = book
        .events()
        .iter()
        .filter(|e| matches!(e, Event::BBO { .. }))
        .count();
    assert_eq!(bbo_count, 0);
}

#[test]
fn bbo_emitted_after_new_best_ask() {
    let mut book = test_book();
    let mut order = incoming(50_100, 100, Side::Sell, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut order);

    assert!(book.events().iter().any(|e| matches!(
        e,
        Event::BBO {
            ask_px, ask_qty, ..
        } if *ask_px == Price(50_100)
            && *ask_qty == Qty(100)
    )));
}

#[test]
fn double_cancel() {
    let mut book = test_book();
    let h = book.insert_resting(49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0);
    assert!(book.cancel_order(h));
    assert!(!book.cancel_order(h));
}

#[test]
fn ioc_on_empty_book() {
    let mut book = test_book();
    let mut order = incoming(50_100, 100, Side::Buy, TimeInForce::IOC, 1);
    process_new_order(&mut book, &mut order);

    let events = book.events();
    let fills = events
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .count();
    assert_eq!(fills, 0);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::OrderDone {
            reason, ..
        } if *reason == OrderDoneReason::Expired
    )));
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::OrderInserted { .. })));
}

#[test]
fn fok_insufficient_liquidity() {
    let mut book = test_book();
    book.insert_resting(50_100, 3, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut order = incoming(50_100, 10, Side::Buy, TimeInForce::FOK, 2);
    process_new_order(&mut book, &mut order);

    let events = book.events();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::OrderFailed {
            reason, ..
        } if *reason == OrderFailureReason::FillOrKill
    )));
    assert!(!events.iter().any(|e| matches!(e, Event::Fill { .. })));
}

#[test]
fn modify_price_loses_time_priority_round_trip() {
    let mut book = test_book();
    let h1 = book.insert_resting(49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0);
    let h2 = book.insert_resting(49_900, 100, Side::Buy, 0, 2, false, 0, 0, 0);
    let h1b = book
        .modify_order_price(h1, 49_800, Side::Buy, 0, 1, false, 1, 0, 0)
        .unwrap();
    let h1c = book
        .modify_order_price(h1b, 49_900, Side::Buy, 0, 1, false, 2, 0, 0)
        .unwrap();
    let tick = book.compression.price_to_index(49_900);
    let level = &book.active_levels[tick as usize];
    assert_eq!(level.head, h2);
    assert_eq!(level.tail, h1c);
}

#[test]
fn modify_qty_down_to_zero_cancels() {
    let mut book = test_book();
    let h = book.insert_resting(49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0);
    assert!(book.modify_order_qty_down(h, 0, 1, 0, 0));
    assert!(!book.orders.get(h).is_active());
    assert_eq!(book.best_bid_tick, rsx_types::NONE);
}

#[test]
fn reduce_only_clamped_to_position() {
    let mut book = test_book();
    book.insert_resting(50_100, 5, Side::Buy, 0, 2, false, 0, 0, 0);
    let mut sell = incoming(50_100, 5, Side::Sell, TimeInForce::GTC, 1);
    process_new_order(&mut book, &mut sell);

    book.insert_resting(50_050, 5, Side::Sell, 0, 3, false, 0, 0, 0);
    let mut ro = incoming(50_050, 10, Side::Buy, TimeInForce::GTC, 1);
    ro.reduce_only = true;
    process_new_order(&mut book, &mut ro);

    let events = book.events();
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::OrderFailed { .. })));
    let fill_qty: i64 = events
        .iter()
        .filter_map(|e| match e {
            Event::Fill { qty, .. } => Some(qty.0),
            _ => None,
        })
        .sum();
    assert_eq!(fill_qty, 5);
}

fn taker_terminals(book: &Orderbook) -> Vec<Option<OrderDoneReason>> {
    book.events()
        .iter()
        .filter_map(|e| match e {
            Event::OrderDone { handle, reason, .. } if *handle == NONE => Some(Some(*reason)),
            Event::OrderFailed { .. } => Some(None),
            _ => None,
        })
        .collect()
}

#[test]
fn every_path_ends_in_exactly_one_done_or_failed() {
    let mut book = test_book();
    book.insert_resting(50_100, 10, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut full = incoming(50_100, 10, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut full);
    assert_eq!(taker_terminals(&book), [Some(OrderDoneReason::Filled)]);

    let mut book = test_book();
    book.insert_resting(50_100, 5, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut ioc = incoming(50_100, 10, Side::Buy, TimeInForce::IOC, 2);
    process_new_order(&mut book, &mut ioc);
    assert_eq!(taker_terminals(&book), [Some(OrderDoneReason::Expired)]);

    let mut book = test_book();
    book.insert_resting(50_100, 5, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut post_only = incoming(50_100, 10, Side::Buy, TimeInForce::GTC, 2);
    post_only.post_only = true;
    process_new_order(&mut book, &mut post_only);
    assert_eq!(taker_terminals(&book), [Some(OrderDoneReason::PostOnly)]);

    let mut book = test_book();
    book.insert_resting(50_100, 5, Side::Sell, 0, 1, false, 0, 0, 0);
    let mut fok = incoming(50_100, 10, Side::Buy, TimeInForce::FOK, 2);
    process_new_order(&mut book, &mut fok);
    assert_eq!(taker_terminals(&book), [None]);

    let mut book = test_book();
    let mut resting = incoming(50_000, 10, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut resting);
    assert!(taker_terminals(&book).is_empty());
}

const CAP: usize = MAX_MAKERS_PER_ORDER;
const EXTRA: usize = 10;

fn sweep_book(side: Side, levels: i64) -> Orderbook {
    let mut book = Orderbook::new(test_config(), (CAP + EXTRA) as u32, 50_000);
    for i in 0..(CAP + EXTRA) {
        let offset = i as i64 % levels;
        let price = match side {
            Side::Sell => 50_100 + offset,
            Side::Buy => 49_900 - offset,
        };
        book.insert_resting(price, 1, side, 0, 1, false, 0, 0, i as u64);
    }
    book
}

fn count_fills(book: &Orderbook) -> usize {
    book.events()
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .count()
}

fn taker_done(book: &Orderbook) -> Vec<(OrderDoneReason, i64, i64)> {
    book.events()
        .iter()
        .filter_map(|e| match e {
            Event::OrderDone {
                handle,
                reason,
                filled_qty,
                remaining_qty,
                ..
            } if *handle == NONE => Some((*reason, filled_qty.0, remaining_qty.0)),
            _ => None,
        })
        .collect()
}

#[test]
fn sweep_past_maker_cap_ends_in_one_match_limit() {
    for levels in [1, 20] {
        for tif in [TimeInForce::GTC, TimeInForce::IOC] {
            let mut book = sweep_book(Side::Sell, levels);
            let total = (CAP + EXTRA) as i64;
            let mut order = incoming(50_200, total, Side::Buy, tif, 2);
            process_new_order(&mut book, &mut order);

            assert_eq!(count_fills(&book), CAP);
            assert_eq!(
                taker_done(&book),
                [(OrderDoneReason::MatchLimit, CAP as i64, EXTRA as i64)],
            );
            assert!(!book
                .events()
                .iter()
                .any(|e| matches!(e, Event::OrderInserted { .. })));
            assert_eq!(book.current_bbo().bid_qty, Qty(0));
            assert!(book.events().len() <= MAX_EVENTS);

            let mut rest = incoming(50_200, EXTRA as i64, Side::Buy, TimeInForce::IOC, 2);
            process_new_order(&mut book, &mut rest);
            assert_eq!(
                taker_done(&book),
                [(OrderDoneReason::Filled, EXTRA as i64, 0)],
            );
            assert_eq!(book.best_ask_tick, NONE);
        }
    }
}

#[test]
fn sell_sweep_past_maker_cap_ends_in_one_match_limit() {
    let mut book = sweep_book(Side::Buy, 1);
    let total = (CAP + EXTRA) as i64;
    let mut order = incoming(49_800, total, Side::Sell, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    assert_eq!(count_fills(&book), CAP);
    assert_eq!(
        taker_done(&book),
        [(OrderDoneReason::MatchLimit, CAP as i64, EXTRA as i64)],
    );
    assert_eq!(book.current_bbo().ask_qty, Qty(0));
}

#[test]
fn fill_or_kill_needing_more_than_the_maker_cap_fails() {
    for levels in [1, 20] {
        let mut book = sweep_book(Side::Sell, levels);
        let mut too_big = incoming(50_200, (CAP + EXTRA) as i64, Side::Buy, TimeInForce::FOK, 2);
        process_new_order(&mut book, &mut too_big);
        assert_eq!(taker_terminals(&book), [None]);
        assert_eq!(count_fills(&book), 0);

        let mut at_cap = incoming(50_200, CAP as i64, Side::Buy, TimeInForce::FOK, 2);
        process_new_order(&mut book, &mut at_cap);
        assert_eq!(count_fills(&book), CAP);
        assert_eq!(
            taker_done(&book),
            [(OrderDoneReason::Filled, CAP as i64, 0)],
        );
    }
}

#[test]
fn residual_after_exactly_the_maker_cap_still_rests() {
    let mut book = Orderbook::new(test_config(), CAP as u32 + 1, 50_000);
    for i in 0..CAP {
        book.insert_resting(50_100, 1, Side::Sell, 0, 1, false, 0, 0, i as u64);
    }
    let mut order = incoming(50_100, CAP as i64 + 5, Side::Buy, TimeInForce::GTC, 2);
    process_new_order(&mut book, &mut order);

    assert_eq!(count_fills(&book), CAP);
    assert!(taker_done(&book).is_empty());
    assert_eq!(book.current_bbo().bid_qty, Qty(5));
}
