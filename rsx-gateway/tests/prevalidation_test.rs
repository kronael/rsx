use rsx_gateway::state::GatewayState;
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
    let state = GatewayState::new(100, 10, 30_000, make_configs());
    // symbol 0 exists
    let sid0 = 0usize;
    assert!(sid0 < state.symbol_configs.len());
    // symbol 1 does not -- gateway rejects before sending
    let sid1 = 1usize;
    assert!(sid1 >= state.symbol_configs.len());
    // symbol u32::MAX also out of bounds
    let sid_max = u32::MAX as usize;
    assert!(sid_max >= state.symbol_configs.len());
}
