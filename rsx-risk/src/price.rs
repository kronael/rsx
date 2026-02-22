use crate::types::BboUpdate;

/// RISK.md §4. Per-symbol index price.
#[derive(Clone, Debug, Default)]
pub struct IndexPrice {
    pub price: i64,
    pub valid: bool,
}

/// RISK.md §4. Size-weighted mid.
pub fn calculate_index(
    bid_px: i64,
    bid_qty: i64,
    ask_px: i64,
    ask_qty: i64,
    last_index: i64,
) -> i64 {
    let total = bid_qty + ask_qty;
    if total == 0 {
        return last_index;
    }
    if bid_qty == 0 {
        return ask_px;
    }
    if ask_qty == 0 {
        return bid_px;
    }
    ((bid_px as i128 * ask_qty as i128
        + ask_px as i128 * bid_qty as i128)
        / total as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

impl IndexPrice {
    pub fn update_from_bbo(&mut self, bbo: &BboUpdate) {
        self.price = calculate_index(
            bbo.bid_px,
            bbo.bid_qty,
            bbo.ask_px,
            bbo.ask_qty,
            self.price,
        );
        self.valid = true;
    }
}
