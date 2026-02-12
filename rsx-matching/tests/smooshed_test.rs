use rsx_book::book::Orderbook;
use rsx_book::event::Event;
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

fn make_order(
    price: i64,
    qty: i64,
    side: Side,
    user_id: u32,
    oid_lo: u64,
    ts: u64,
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
        timestamp_ns: ts,
        order_id_hi: 0,
        order_id_lo: oid_lo,
    }
}

/// Two sells at different prices map to the same
/// compressed slot. A buy at the lower price should
/// only match the sell at that exact price, not the
/// higher-priced sell in the same slot.
#[test]
fn smooshed_level_scan_checks_exact_price() {
    // Use a mid_price where zone 1 has compression=10,
    // so prices 10 ticks apart share a slot.
    // mid=50000, tick=10. Zone 0 boundary = 5% =
    // 50000*5/(100*10) = 250 ticks.
    // Zone 1 starts at 250 ticks from mid, compression=10.
    // Two prices in zone 1 that differ by < 10 ticks
    // map to the same slot.
    //
    // Prices at mid + 260*10 = 52600 and mid + 261*10 = 52610
    // Both are 260 and 261 ticks from mid, zone 1
    // (250..750), compression 10.
    // floor((260-250)/10)=1, floor((261-250)/10)=1 -> same slot
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    let price_a = 50_000 + 260 * 10; // 52600
    let price_b = 50_000 + 261 * 10; // 52610

    // Verify they map to the same index
    let idx_a =
        book.compression.price_to_index(price_a);
    let idx_b =
        book.compression.price_to_index(price_b);
    assert_eq!(
        idx_a, idx_b,
        "prices should map to same slot"
    );

    // Place sell at price_b (higher)
    let mut sell_b = make_order(
        price_b,
        10,
        Side::Sell,
        2,
        2,
        1000,
    );
    process_new_order(&mut book, &mut sell_b);

    // Place sell at price_a (lower)
    let mut sell_a = make_order(
        price_a,
        10,
        Side::Sell,
        3,
        3,
        2000,
    );
    process_new_order(&mut book, &mut sell_a);

    // Buy at price_a: should only match sell_a, not
    // sell_b (which is priced higher than the buy).
    let mut buy = make_order(
        price_a,
        10,
        Side::Buy,
        1,
        1,
        3000,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    let fills: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Fill {
                maker_user_id,
                price,
                ..
            } => Some((*maker_user_id, price.0)),
            _ => None,
        })
        .collect();

    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].0, 3); // matched sell_a
    assert_eq!(fills[0].1, price_a);
}

/// A buy at a price that doesn't match any maker in the
/// smooshed slot should skip all orders and rest.
#[test]
fn smooshed_level_skips_non_matching_price() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    let price_high = 50_000 + 261 * 10; // 52610

    // Sell at price_high in a smooshed zone
    let mut sell = make_order(
        price_high,
        10,
        Side::Sell,
        2,
        2,
        1000,
    );
    process_new_order(&mut book, &mut sell);

    // Buy at lower price in same slot: 52600
    let price_low = 50_000 + 260 * 10;
    assert_eq!(
        book.compression.price_to_index(price_high),
        book.compression.price_to_index(price_low),
    );

    // The buy at price_low should NOT match the sell at
    // price_high because price_high > price_low.
    let mut buy = make_order(
        price_low,
        10,
        Side::Buy,
        1,
        1,
        2000,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    let fills: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::Fill { .. }))
        .collect();
    assert_eq!(
        fills.len(),
        0,
        "should not match across price boundary"
    );
}

/// Within a smooshed slot, orders at the same exact price
/// are filled in time priority (FIFO by insertion order).
#[test]
fn smooshed_level_time_priority_within_slot() {
    let mut book = Orderbook::new(
        test_config(),
        4096,
        50_000,
    );

    let price = 50_000 + 260 * 10; // 52600

    // Three sells at same price, different timestamps
    for i in 0..3u32 {
        let mut sell = make_order(
            price,
            10,
            Side::Sell,
            10 + i,
            10 + i as u64,
            1000 + i as u64,
        );
        process_new_order(&mut book, &mut sell);
    }

    // Buy sweeps all three
    let mut buy = make_order(
        price,
        30,
        Side::Buy,
        1,
        1,
        5000,
    );
    process_new_order(&mut book, &mut buy);
    let events = book.events();

    let fill_order: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Fill {
                maker_user_id, ..
            } => Some(*maker_user_id),
            _ => None,
        })
        .collect();

    assert_eq!(fill_order, vec![10, 11, 12]);
}
