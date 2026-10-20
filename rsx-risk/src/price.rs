use crate::shard::RiskShard;
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
    let total = bid_qty as i128 + ask_qty as i128;
    if total == 0 {
        return last_index;
    }
    if bid_qty == 0 {
        return ask_px;
    }
    if ask_qty == 0 {
        return bid_px;
    }
    ((bid_px as i128 * ask_qty as i128 + ask_px as i128 * bid_qty as i128) / total)
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

impl IndexPrice {
    pub fn update_from_bbo(&mut self, bbo: &BboUpdate) {
        self.price = calculate_index(bbo.bid_px, bbo.bid_qty, bbo.ask_px, bbo.ask_qty, self.price);
        self.valid = true;
    }
}

impl RiskShard {
    /// RISK.md §4. Update index price from BBO.
    pub fn process_bbo(&mut self, bbo: &BboUpdate) {
        let sid = bbo.symbol_id as usize;
        if sid >= self.index_prices.len() {
            return;
        }
        self.index_prices[sid].update_from_bbo(bbo);
        self.mark_dirty = true;
    }

    pub fn update_mark(&mut self, symbol_id: u32, price: i64) {
        if (symbol_id as usize) >= self.mark_prices.len() {
            return;
        }
        self.mark_prices[symbol_id as usize] = price;
        self.mark_dirty = true;
    }

    pub fn stash_bbo(&mut self, bbo: BboUpdate) {
        let sid = bbo.symbol_id as usize;
        if sid >= self.stashed_bbo.len() {
            return;
        }
        self.stashed_bbo[sid] = Some(bbo);
    }

    pub fn drain_stashed_bbos(&mut self, now_ns: u64) {
        for sid in 0..self.max_symbols {
            if let Some(bbo) = self.stashed_bbo[sid].take() {
                self.process_bbo(&bbo);
                // RISK.md §7: margin scan on every BBO.
                // Index once to get count, then re-index per
                // iteration — each borrow is short-lived (Copy
                // u32) so no conflict with the &mut self call.
                let n = self.exposure.users_for_symbol(sid).len();
                for i in 0..n {
                    let user_id = self.exposure.users_for_symbol(sid)[i];
                    self.check_liquidation_for(user_id, now_ns);
                }
            }
        }
    }

    /// Rebuild fallback_mark_prices from mark_prices,
    /// filling zeros with valid index prices. Called once
    /// per price update, not per order.
    pub(crate) fn rebuild_fallback(&mut self) {
        self.fallback_mark_prices.copy_from_slice(&self.mark_prices);
        for (i, v) in self.fallback_mark_prices.iter_mut().enumerate() {
            if *v == 0 {
                if let Some(idx) = self.index_prices.get(i) {
                    if idx.valid && idx.price > 0 {
                        *v = idx.price;
                    }
                }
            }
        }
        self.mark_dirty = false;
    }
}
