/// RISK.md §2. All fields i64 fixed-point.
#[derive(Clone, Debug, Default)]
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
                let avg = self.short_entry_cost
                    / self.short_qty;
                self.realized_pnl +=
                    close_qty * (avg - price);
                self.short_qty -= close_qty;
                self.short_entry_cost -= avg * close_qty;
                let remaining = qty - close_qty;
                if remaining > 0 {
                    // Flip to long
                    self.long_qty += remaining;
                    self.long_entry_cost +=
                        price * remaining;
                }
            } else {
                // Accumulate long
                self.long_qty += qty;
                self.long_entry_cost += price * qty;
            }
        } else {
            // Sell fill
            if self.long_qty > 0 {
                // Opposing: reduce long
                let close_qty = qty.min(self.long_qty);
                let avg = self.long_entry_cost
                    / self.long_qty;
                self.realized_pnl +=
                    close_qty * (price - avg);
                self.long_qty -= close_qty;
                self.long_entry_cost -= avg * close_qty;
                let remaining = qty - close_qty;
                if remaining > 0 {
                    // Flip to short
                    self.short_qty += remaining;
                    self.short_entry_cost +=
                        price * remaining;
                }
            } else {
                // Accumulate short
                self.short_qty += qty;
                self.short_entry_cost += price * qty;
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
        self.net_qty().abs() * mark_price
    }

    pub fn avg_entry(&self) -> i64 {
        let nq = self.net_qty();
        if nq > 0 {
            self.long_entry_cost / self.long_qty
        } else if nq < 0 {
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
        nq * (mark_price - self.avg_entry())
    }

    pub fn is_empty(&self) -> bool {
        self.long_qty == 0 && self.short_qty == 0
    }
}
