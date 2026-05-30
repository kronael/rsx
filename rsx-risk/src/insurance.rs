/// LIQUIDATOR.md §9. Insurance fund for socialized losses.

#[derive(Clone, Debug, Default)]
#[repr(C, align(64))]
pub struct InsuranceFund {
    pub symbol_id: u32,
    pub balance: i64,
    pub version: u64,
}

impl InsuranceFund {
    pub fn new(symbol_id: u32, initial_balance: i64) -> Self {
        Self {
            symbol_id,
            balance: initial_balance,
            version: 0,
        }
    }

    /// Balance MAY go negative (insurance exhausted -> loss is
    /// socialized further; the negative balance signals depletion).
    /// Saturating so a pathological loss can't panic the shard in
    /// debug builds or wrap in release (matches the i128-clamp
    /// discipline in `process_socialized_loss`).
    pub fn deduct(&mut self, amount: i64) {
        self.balance = self.balance.saturating_sub(amount);
        self.version += 1;
    }

    pub fn add(&mut self, amount: i64) {
        self.balance = self.balance.saturating_add(amount);
        self.version += 1;
    }
}
