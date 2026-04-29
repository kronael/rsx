/// RISK.md §2. Frozen margin is derived from
/// `RiskShard::frozen_for_user`; not stored here.
#[derive(Clone, Debug, Default)]
#[repr(C, align(64))]
pub struct Account {
    pub user_id: u32,
    pub collateral: i64,
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

    /// RISK.md §1. Negative fee = rebate credited.
    pub fn deduct_fee(&mut self, fee: i64) {
        self.collateral -= fee;
        self.version += 1;
    }
}
