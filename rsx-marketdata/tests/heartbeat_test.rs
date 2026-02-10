use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;
use std::thread;
use std::time::Duration;

#[test]
fn heartbeat_broadcast_sends_to_all_clients() {
    let cfg = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(10, cfg, 100, 50000);
    let conn1 = state.add_connection();
    let conn2 = state.add_connection();

    state.broadcast_heartbeat(12345);

    let msgs1 = state.drain_outbound(conn1);
    let msgs2 = state.drain_outbound(conn2);

    assert_eq!(msgs1.len(), 1);
    assert_eq!(msgs2.len(), 1);
    assert_eq!(msgs1[0], "{\"H\":[12345]}");
    assert_eq!(msgs2[0], "{\"H\":[12345]}");
}

#[test]
fn update_heartbeat_refreshes_timestamp() {
    let cfg = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(10, cfg, 100, 50000);
    let conn_id = state.add_connection();

    thread::sleep(Duration::from_millis(10));

    state.update_heartbeat(conn_id);

    let timeout_ns = 5_000_000;
    let timed_out = state.check_timeouts(timeout_ns);
    assert_eq!(timed_out.len(), 0);
}

#[test]
fn check_timeouts_removes_stale_connections() {
    let cfg = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(10, cfg, 100, 50000);
    let conn1 = state.add_connection();
    let conn2 = state.add_connection();

    thread::sleep(Duration::from_millis(20));

    state.update_heartbeat(conn2);

    let timeout_ns = 10_000_000;
    let timed_out = state.check_timeouts(timeout_ns);

    assert_eq!(timed_out.len(), 1);
    assert_eq!(timed_out[0], conn1);

    let msgs1 = state.drain_outbound(conn1);
    assert_eq!(msgs1.len(), 0);
}

#[test]
fn heartbeat_timeout_removes_subscription() {
    let cfg = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(10, cfg, 100, 50000);
    let conn_id = state.add_connection();
    state.subscribe(conn_id, 1, 3, 10);

    let clients_before = state.clients_for_symbol(1);
    assert_eq!(clients_before.len(), 1);

    thread::sleep(Duration::from_millis(20));

    let timeout_ns = 10_000_000;
    state.check_timeouts(timeout_ns);

    let clients_after = state.clients_for_symbol(1);
    assert_eq!(clients_after.len(), 0);
}

#[test]
fn heartbeat_within_timeout_keeps_connection() {
    let cfg = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(10, cfg, 100, 50000);
    let conn_id = state.add_connection();

    thread::sleep(Duration::from_millis(5));
    state.update_heartbeat(conn_id);

    thread::sleep(Duration::from_millis(5));
    state.update_heartbeat(conn_id);

    let timeout_ns = 20_000_000;
    let timed_out = state.check_timeouts(timeout_ns);
    assert_eq!(timed_out.len(), 0);

    let msgs = state.drain_outbound(conn_id);
    assert_eq!(msgs.len(), 0);
}

#[test]
fn multiple_clients_timeout_independently() {
    let cfg = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(10, cfg, 100, 50000);
    let conn1 = state.add_connection();
    let conn2 = state.add_connection();
    let conn3 = state.add_connection();

    thread::sleep(Duration::from_millis(10));
    state.update_heartbeat(conn2);

    thread::sleep(Duration::from_millis(10));
    state.update_heartbeat(conn3);

    let timeout_ns = 15_000_000;
    let timed_out = state.check_timeouts(timeout_ns);

    assert_eq!(timed_out.len(), 1);
    assert_eq!(timed_out[0], conn1);
}
