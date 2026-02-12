use rsx_marketdata::shadow::ShadowBook;
use rsx_marketdata::state::MarketDataState;
use rsx_marketdata::subscription::CHANNEL_DEPTH;
use rsx_types::SymbolConfig;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn base_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

/// MD19: Snapshot is point-in-time consistent. All levels
/// in a single snapshot reflect the same seq.
#[test]
fn snapshot_point_in_time_consistent() {
    let mut book = ShadowBook::new(config(), 1024, 50000);
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(49980, 30, 0, 2, 1001);
    book.apply_insert(50010, 40, 1, 3, 1002);
    let expected_seq = book.seq();

    let snap = book.derive_l2_snapshot(10);
    // All levels share the same seq
    assert_eq!(snap.seq, expected_seq);
    assert_eq!(snap.bids.len(), 2);
    assert_eq!(snap.asks.len(), 1);
    // Timestamp matches last event
    assert_eq!(snap.timestamp_ns, 1002);
}

/// MD24: Server sends B snapshot on subscribe before any
/// D deltas. Verify send_snapshot_to_client clears any
/// pending deltas and queues a B frame as first message.
#[test]
fn snapshot_before_deltas_on_subscribe() {
    let mut state = MarketDataState::new(
        4, base_config(), 100, 50000,
    );
    let conn = state.add_connection();
    state.subscribe(conn, 1, CHANNEL_DEPTH, 10);

    // Push a delta before snapshot (simulates race)
    state.push_to_client(
        conn,
        "{\"D\":[1,0,49990,50,1,1000,1]}".into(),
        100,
    );

    // send_snapshot_to_client clears queue, pushes
    // snapshot as first message (no book = empty snap)
    state.send_snapshot_to_client(conn, 1, 10, 100);

    let msgs = state.drain_outbound(conn);
    assert_eq!(msgs.len(), 1);
    // First (and only) message must be a B snapshot
    assert!(
        msgs[0].starts_with("{\"B\":["),
        "first message must be snapshot, got: {}",
        msgs[0],
    );
}
