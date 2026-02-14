/// RISK.md §2. All fields i64 fixed-point.
#[derive(Clone, Debug, Default)]
#[repr(C, align(64))]
pub struct Position {
    pub user_id: u32,
    pub symbol_id: u32,
    pub long_qty: i64,
    pub short_qty: i64,
    pub long_entry_cost: i64,
    pub short_entry_cost: i64,
    pub realized_pnl: i64,
    pub last_fill_seq: u64,
    pub version: u64,
}

impl Position {
    pub fn new(user_id: u32, symbol_id: u32) -> Self {
        Self {
            user_id,
            symbol_id,
            ..Default::default()
        }
    }

    /// RISK.md §1. side: 0=Buy, 1=Sell.
    pub fn apply_fill(
        &mut self,
        side: u8,
        price: i64,
        qty: i64,
        seq: u64,
    ) {
        if side == 0 {
            // Buy fill
            if self.short_qty > 0 {
                // Opposing: reduce short
                let close_qty = qty.min(self.short_qty);
                let close_cost = (self.short_entry_cost
                    as i128
                    * close_qty as i128
                    / self.short_qty as i128)
                    as i64;
                self.realized_pnl += (close_cost as i128
                    - price as i128 * close_qty as i128)
                    as i64;
                self.short_qty -= close_qty;
                self.short_entry_cost -= close_cost;
                let remaining = qty - close_qty;
                if remaining > 0 {
                    self.long_qty += remaining;
                    self.long_entry_cost +=
                        (price as i128 * remaining as i128)
                            as i64;
                }
            } else {
                // Accumulate long
                self.long_qty += qty;
                self.long_entry_cost +=
                    (price as i128 * qty as i128) as i64;
            }
        } else {
            // Sell fill
            if self.long_qty > 0 {
                // Opposing: reduce long
                let close_qty = qty.min(self.long_qty);
                let close_cost = (self.long_entry_cost
                    as i128
                    * close_qty as i128
                    / self.long_qty as i128)
                    as i64;
                self.realized_pnl +=
                    (price as i128 * close_qty as i128
                        - close_cost as i128)
                        as i64;
                self.long_qty -= close_qty;
                self.long_entry_cost -= close_cost;
                let remaining = qty - close_qty;
                if remaining > 0 {
                    self.short_qty += remaining;
                    self.short_entry_cost +=
                        (price as i128 * remaining as i128)
                            as i64;
                }
            } else {
                // Accumulate short
                self.short_qty += qty;
                self.short_entry_cost +=
                    (price as i128 * qty as i128) as i64;
            }
        }
        self.version += 1;
        self.last_fill_seq = seq;
    }

    pub fn net_qty(&self) -> i64 {
        self.long_qty - self.short_qty
    }

    /// RISK.md §3.
    pub fn notional(&self, mark_price: i64) -> i64 {
        let v = self.net_qty().abs() as i128
            * mark_price as i128;
        i64::try_from(v).unwrap_or(if v > 0 {
            i64::MAX
        } else {
            i64::MIN
        })
    }

    pub fn avg_entry(&self) -> i64 {
        let nq = self.net_qty();
        if nq > 0 && self.long_qty != 0 {
            self.long_entry_cost / self.long_qty
        } else if nq < 0 && self.short_qty != 0 {
            self.short_entry_cost / self.short_qty
        } else {
            0
        }
    }

    /// RISK.md §3.
    pub fn unrealized_pnl(
        &self,
        mark_price: i64,
    ) -> i64 {
        let nq = self.net_qty();
        if nq == 0 {
            return 0;
        }
        let v = nq as i128
            * (mark_price - self.avg_entry()) as i128;
        i64::try_from(v).unwrap_or(if v > 0 {
            i64::MAX
        } else {
            i64::MIN
        })
    }

    pub fn is_empty(&self) -> bool {
        self.long_qty == 0 && self.short_qty == 0
    }
}
