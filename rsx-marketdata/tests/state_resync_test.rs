use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;

fn base_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    }
}

#[test]
fn empty_book_snapshot_msg() {
    let state = MarketDataState::new(4, base_config(), 100, 50_000);
    let msg = state.snapshot_msg(1, 10).unwrap();
    assert_eq!(msg, "{\"B\":[1,[],[],0,0]}");
}

#[test]
fn send_snapshot_clears_queue_on_backpressure() {
    let mut state = MarketDataState::new(4, base_config(), 100, 50_000);
    let conn_id = state.add_connection();

    // Fill outbound to capacity 1
    assert!(state.push_to_client(conn_id, "x".to_string(), 1));
    assert!(!state.push_to_client(conn_id, "y".to_string(), 1));

    state.send_snapshot_to_client(conn_id, 1, 10, 10);

    let msgs = state.drain_outbound(conn_id);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0], "{\"B\":[1,[],[],0,0]}");
}
