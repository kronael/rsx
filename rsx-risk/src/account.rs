/// RISK.md §2.
#[derive(Clone, Debug, Default)]
#[repr(C, align(64))]
pub struct Account {
    pub user_id: u32,
    pub collateral: i64,
    pub frozen_margin: i64,
    pub version: u64,
}

impl Account {
    pub fn new(user_id: u32, collateral: i64) -> Self {
        Self {
            user_id,
            collateral,
            ..Default::default()
        }
    }

    pub fn freeze_margin(&mut self, amount: i64) {
        self.frozen_margin += amount;
        self.version += 1;
    }

    pub fn release_margin(&mut self, amount: i64) {
        self.frozen_margin -= amount;
        self.version += 1;
    }

    /// RISK.md §1. Negative fee = rebate credited.
    pub fn deduct_fee(&mut self, fee: i64) {
        self.collateral -= fee;
        self.version += 1;
    }
}
