use rsx_gateway::convert::*;
use rsx_types::SymbolConfig;

fn btc_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1,
    }
}

fn eth_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 2,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 10,
        lot_size: 10,
    }
}

#[test]
fn price_float_to_fixed_point_correct() {
    // 50000.00 with 2 decimals -> 5000000, tick_size=100
    // 5000000 % 100 == 0
    let r = price_to_fixed(50000.00, &btc_config());
    assert_eq!(r, Some(5000000));
}

#[test]
fn qty_float_to_fixed_point_correct() {
    // 1.5 with 3 decimals -> 1500, lot_size=1
    let r = qty_to_fixed(1.5, &btc_config());
    assert_eq!(r, Some(1500));
}

#[test]
fn price_fractional_tick_rejected() {
    // 50000.05 -> 5000005, 5000005 % 100 != 0
    let r = price_to_fixed(50000.05, &btc_config());
    assert_eq!(r, None);
}

#[test]
fn qty_fractional_lot_rejected() {
    // 1.55 with 2 decimals -> 155, 155 % 10 != 0
    let r = qty_to_fixed(1.55, &eth_config());
    assert_eq!(r, None);
}

#[test]
fn validate_tick_alignment_pass() {
    assert!(validate_tick_alignment(5000000, 100));
}

#[test]
fn validate_tick_alignment_fail() {
    assert!(!validate_tick_alignment(5000001, 100));
}

#[test]
fn validate_lot_alignment_pass() {
    assert!(validate_lot_alignment(1500, 1));
    assert!(validate_lot_alignment(150, 10));
}

#[test]
fn validate_lot_alignment_fail() {
    assert!(!validate_lot_alignment(155, 10));
}

#[test]
fn price_zero_rejected() {
    assert_eq!(price_to_fixed(0.0, &btc_config()), None);
}

#[test]
fn qty_zero_rejected() {
    assert_eq!(qty_to_fixed(0.0, &btc_config()), None);
}
