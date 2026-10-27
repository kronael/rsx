use rsx_gateway::state::GatewayState;
use rsx_gateway::symbols::build_table;
use rsx_types::SymbolConfig;

fn make_configs() -> Vec<SymbolConfig> {
    vec![SymbolConfig {
        symbol_id: 0,
        price_decimals: 8,
        qty_decimals: 8,
        tick_size: 10,
        lot_size: 5,
    }]
}

#[test]
fn symbol_not_found_rejects_early() {
    let state = GatewayState::new(100, 10, 30_000, build_table(make_configs()));
    // symbol 0 exists
    assert!(state.symbols.contains_key(&0));
    // symbol 1 does not -- gateway rejects before sending
    assert!(!state.symbols.contains_key(&1));
    // symbol u32::MAX also out of bounds
    assert!(!state.symbols.contains_key(&u32::MAX));
}
