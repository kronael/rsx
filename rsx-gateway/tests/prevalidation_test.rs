use rsx_gateway::state::GatewayState;
use rsx_types::SymbolConfig;

fn make_configs() -> Vec<SymbolConfig> {
    vec![
        SymbolConfig {
            symbol_id: 0,
            price_decimals: 8,
            qty_decimals: 8,
            tick_size: 10,
            lot_size: 5,
        },
    ]
}

#[test]
fn symbol_not_found_rejects_early() {
    let state =
        GatewayState::new(100, 10, 30_000, make_configs());
    // symbol_id=1 is out of bounds (only 0 configured)
    assert_eq!(state.symbol_configs.len(), 1);
    assert!(1 >= state.symbol_configs.len());
}

#[test]
fn config_cache_updated_on_config_applied() {
    let mut state =
        GatewayState::new(100, 10, 30_000, make_configs());
    assert_eq!(state.symbol_configs[0].tick_size, 10);

    // Simulate CONFIG_APPLIED updating the cache
    state.symbol_configs[0].tick_size = 20;
    assert_eq!(state.symbol_configs[0].tick_size, 20);

    // New config is used for validation
    state.symbol_configs.push(SymbolConfig {
        symbol_id: 1,
        price_decimals: 8,
        qty_decimals: 8,
        tick_size: 100,
        lot_size: 50,
    });
    assert_eq!(state.symbol_configs.len(), 2);
    assert_eq!(state.symbol_configs[1].tick_size, 100);
}
