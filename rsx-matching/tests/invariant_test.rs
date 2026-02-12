use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_types::NONE;
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

/// For every order that gets fills, Fill events must appear
/// before the corresponding OrderDone in the event buffer.
#[test]
fn fills_precede_order_done_always() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    // Place 10 sells, then one buy sweeps them
    for i in 0..10u32 {
        let mut sell = make_order(
            50_000,
            10,
            Side::Sell,
            100 + i,
            100 + i as u64,
        );
        process_new_order(&mut book, &mut sell);
    }

    let mut buy = make_order(
        50_000,
        100,
        Side::Buy,
        1,
        1,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    // For each maker: find its Fill index, find its
    // OrderDone index, assert Fill < Done.
    for maker_id in 100..110u32 {
        let fill_idx = events
            .iter()
            .position(|e| match e {
                Event::Fill {
                    maker_user_id, ..
                } => *maker_user_id == maker_id,
                _ => false,
            });
        let done_idx = events
            .iter()
            .position(|e| match e {
                Event::OrderDone {
                    user_id, ..
                } => *user_id == maker_id,
                _ => false,
            });
        assert!(
            fill_idx.unwrap() < done_idx.unwrap(),
            "fill must precede done for maker {}",
            maker_id,
        );
    }
}

/// Each order gets exactly one completion event
/// (OrderDone or OrderFailed), never two.
#[test]
fn exactly_one_completion_per_order() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    // 20 sells, one buy sweeps all
    for i in 0..20u32 {
        let mut sell = make_order(
            50_000,
            5,
            Side::Sell,
            100 + i,
            100 + i as u64,
        );
        process_new_order(&mut book, &mut sell);
    }

    let mut buy = make_order(
        50_000,
        100,
        Side::Buy,
        1,
        1,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    // Count OrderDone per order_id_lo
    let mut done_counts =
        std::collections::HashMap::<u64, u32>::new();
    for e in events {
        if let Event::OrderDone {
            order_id_lo, ..
        } = e
        {
            *done_counts.entry(*order_id_lo).or_default()
                += 1;
        }
    }

    for (oid, count) in &done_counts {
        assert_eq!(
            *count, 1,
            "order {} got {} completions",
            oid, count,
        );
    }
}

/// Orders at the same price level are filled in FIFO order.
#[test]
fn fifo_within_price_level_verified() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    // Place 5 sells at same price, different users
    let maker_ids: Vec<u32> = (10..15).collect();
    for (i, &uid) in maker_ids.iter().enumerate() {
        let mut sell = make_order(
            50_000,
            10,
            Side::Sell,
            uid,
            uid as u64,
        );
        sell.timestamp_ns = 1000 + i as u64;
        process_new_order(&mut book, &mut sell);
    }

    // Buy sweeps all 5
    let mut buy = make_order(
        50_000,
        50,
        Side::Buy,
        1,
        1,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    // Extract fill order by maker_user_id
    let fill_maker_order: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Fill {
                maker_user_id, ..
            } => Some(*maker_user_id),
            _ => None,
        })
        .collect();

    assert_eq!(fill_maker_order, maker_ids);
}

/// After every operation, best_bid < best_ask (no crossed
/// book), or one/both sides are empty.
#[test]
fn best_bid_ask_coherent_after_every_op() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    let ops: Vec<(i64, Side)> = vec![
        (49_990, Side::Buy),
        (49_980, Side::Buy),
        (50_010, Side::Sell),
        (50_020, Side::Sell),
        // Cross: buy at ask
        (50_010, Side::Buy),
        (49_990, Side::Sell),
    ];

    for (i, (price, side)) in ops.iter().enumerate() {
        let mut order = make_order(
            *price,
            10,
            *side,
            i as u32 + 1,
            i as u64 + 1,
        );
        process_new_order(&mut book, &mut order);

        if book.best_bid_tick != NONE
            && book.best_ask_tick != NONE
        {
            assert!(
                book.best_bid_tick < book.best_ask_tick,
                "crossed book after op {}: bid={} ask={}",
                i,
                book.best_bid_tick,
                book.best_ask_tick,
            );
        }
    }
}

/// After 1M insert/cancel cycles the slab does not leak.
/// We use a smaller slab and verify alloc still works.
#[test]
fn slab_no_leak_after_1m_operations() {
    let mut book = Orderbook::new(
        test_config(),
        1024,
        50_000,
    );

    for i in 0..1_000_000u64 {
        let mut order = make_order(
            50_000,
            10,
            Side::Buy,
            1,
            i,
        );
        process_new_order(&mut book, &mut order);
        let events = book.events();
        let handle = match events[0] {
            Event::OrderInserted { handle, .. } => {
                handle
            }
            _ => continue,
        };
        book.cancel_order(handle);
    }

    // Slab should still be able to allocate (not exhausted)
    let mut order = make_order(
        50_000,
        10,
        Side::Buy,
        1,
        999_999_999,
    );
    process_new_order(&mut book, &mut order);
    let events = book.events();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::OrderInserted { .. }
    )));
}

/// Event sequences within a single process_new_order call
/// are monotonically ordered (Fill before Done before BBO).
#[test]
fn event_seq_monotonic_within_symbol() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    // Place a sell, then a buy to trigger fill sequence
    let mut sell = make_order(
        50_000,
        10,
        Side::Sell,
        2,
        2,
    );
    process_new_order(&mut book, &mut sell);

    let mut buy = make_order(
        50_000,
        10,
        Side::Buy,
        1,
        1,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    // Verify ordering: all Fills before OrderDones,
    // all OrderDones before BBO
    let mut seen_done = false;
    let mut seen_bbo = false;
    for e in events {
        match e {
            Event::Fill { .. } => {
                assert!(
                    !seen_done,
                    "fill after done",
                );
                assert!(
                    !seen_bbo,
                    "fill after bbo",
                );
            }
            Event::OrderDone { .. } => {
                assert!(
                    !seen_bbo,
                    "done after bbo",
                );
                seen_done = true;
            }
            Event::BBO { .. } => {
                seen_bbo = true;
            }
            _ => {}
        }
    }
}
