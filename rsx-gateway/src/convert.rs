use rsx_types::SymbolConfig;

/// Convert float price to fixed-point tick units.
/// Returns None if price doesn't align to tick_size.
pub fn price_to_fixed(
    price_f64: f64,
    config: &SymbolConfig,
) -> Option<i64> {
    let scale = 10f64.powi(config.price_decimals as i32);
    let raw = (price_f64 * scale).round() as i64;
    if raw <= 0 {
        return None;
    }
    if raw % config.tick_size != 0 {
        return None;
    }
    Some(raw)
}

/// Convert float qty to fixed-point lot units.
/// Returns None if qty doesn't align to lot_size.
pub fn qty_to_fixed(
    qty_f64: f64,
    config: &SymbolConfig,
) -> Option<i64> {
    let scale = 10f64.powi(config.qty_decimals as i32);
    let raw = (qty_f64 * scale).round() as i64;
    if raw <= 0 {
        return None;
    }
    if raw % config.lot_size != 0 {
        return None;
    }
    Some(raw)
}

/// Validate price alignment to tick size.
pub fn validate_tick_alignment(
    price: i64,
    tick_size: i64,
) -> bool {
    price > 0 && price % tick_size == 0
}

/// Validate qty alignment to lot size.
pub fn validate_lot_alignment(
    qty: i64,
    lot_size: i64,
) -> bool {
    qty > 0 && qty % lot_size == 0
}
