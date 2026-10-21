/// Validate price alignment to tick size.
pub fn validate_tick_alignment(price: i64, tick_size: i64) -> bool {
    price > 0 && price % tick_size == 0
}

/// Validate qty alignment to lot size.
pub fn validate_lot_alignment(qty: i64, lot_size: i64) -> bool {
    qty > 0 && qty % lot_size == 0
}
