use rsx_types::*;

#[test]
fn price_newtype_ordering_correct() {
    assert!(Price(100) < Price(200));
    assert!(Price(200) > Price(100));
    assert_eq!(Price(100), Price(100));
}

#[test]
fn qty_newtype_arithmetic() {
    let a = Qty(100);
    let b = Qty(50);
    assert_eq!(Qty(a.0 - b.0), Qty(50));
    assert_eq!(Qty(a.0 + b.0), Qty(150));
}

#[test]
fn side_repr_values() {
    assert_eq!(Side::Buy as u8, 0);
    assert_eq!(Side::Sell as u8, 1);
}

#[test]
fn tif_repr_values() {
    assert_eq!(TimeInForce::GTC as u8, 0);
    assert_eq!(TimeInForce::IOC as u8, 1);
    assert_eq!(TimeInForce::FOK as u8, 2);
}

#[test]
fn validate_order_accepts_aligned() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(validate_order(&config, Price(500), Qty(3000)));
}

#[test]
fn validate_order_rejects_price_not_aligned() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(!validate_order(&config, Price(501), Qty(3000)));
}

#[test]
fn validate_order_rejects_qty_not_aligned() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(!validate_order(&config, Price(500), Qty(3001)));
}

#[test]
fn validate_order_rejects_zero_qty() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(!validate_order(&config, Price(500), Qty(0)));
}

#[test]
fn validate_order_rejects_zero_price() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(!validate_order(&config, Price(0), Qty(1000)));
}

#[test]
fn validate_order_rejects_negative_price() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(!validate_order(&config, Price(-100), Qty(1000)));
}

#[test]
fn validate_order_rejects_negative_qty() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 1000,
    };
    assert!(!validate_order(&config, Price(100), Qty(-1000)));
}

#[test]
fn none_sentinel_is_u32_max() {
    assert_eq!(NONE, u32::MAX);
}
