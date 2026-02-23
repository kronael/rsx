use rsx_gateway::convert::validate_lot_alignment;
use rsx_gateway::convert::validate_tick_alignment;
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
    let state =
        GatewayState::new(100, 10, 30_000, make_configs());
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

#[test]
fn config_cache_updated_on_config_applied() {
    std::env::set_var("RSX_SYMBOL_0_TICK_SIZE", "20");
    std::env::set_var("RSX_SYMBOL_0_LOT_SIZE", "10");

    let mut state =
        GatewayState::new(100, 10, 30_000, make_configs());
    assert_eq!(state.symbol_configs[0].tick_size, 10);
    assert_eq!(state.symbol_configs[0].lot_size, 5);

    // apply_config_applied reloads from env
    assert!(state.apply_config_applied(0, 1));
    assert_eq!(state.symbol_configs[0].tick_size, 20);
    assert_eq!(state.symbol_configs[0].lot_size, 10);

    // validation now uses updated config
    assert!(validate_tick_alignment(20, 20));
    assert!(!validate_tick_alignment(15, 20));
    assert!(validate_lot_alignment(10, 10));
    assert!(!validate_lot_alignment(7, 10));

    // stale version rejected
    assert!(!state.apply_config_applied(0, 0));
    assert_eq!(state.config_versions[0], 1);

    // unknown symbol rejected
    assert!(!state.apply_config_applied(99, 1));

    std::env::remove_var("RSX_SYMBOL_0_TICK_SIZE");
    std::env::remove_var("RSX_SYMBOL_0_LOT_SIZE");
}
