use crate::liquidation::SocializedLoss;
use crate::persist::LiquidationRecord;
use crate::persist::PersistEvent;
use crate::shard::RiskShard;
use tracing::warn;

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

impl RiskShard {
    /// LIQUIDATOR.md §9. Process socialized loss event.
    /// Deduct loss from insurance fund, persist events.
    pub(crate) fn process_socialized_loss(&mut self, loss: &SocializedLoss, now_ns: u64) {
        let loss_amount = i64::try_from(loss.qty as i128 * loss.price as i128).unwrap_or(i64::MAX);

        // Ensure insurance fund exists for symbol
        let fund = self
            .insurance_funds
            .entry(loss.symbol_id)
            .or_insert_with(|| InsuranceFund::new(loss.symbol_id, 0));

        // Deduct from insurance fund (balance can go negative)
        fund.deduct(loss_amount);

        // Clone fund for persistence before releasing borrow
        let fund_snapshot = fund.clone();

        warn!(
            "socialized loss: user={} symbol={} \
             round={} qty={} price={} loss={}",
            loss.user_id, loss.symbol_id, loss.round, loss.qty, loss.price, loss_amount
        );

        // Persist liquidation event with status=3 (socialized)
        self.push_persist(PersistEvent::Liquidation(LiquidationRecord {
            user_id: loss.user_id,
            symbol_id: loss.symbol_id,
            round: loss.round,
            side: loss.side,
            price: loss.price,
            qty: loss.qty,
            slippage_bps: 0,
            status: 3,
            timestamp_ns: now_ns,
        }));

        // Persist updated insurance fund
        self.push_persist(PersistEvent::InsuranceFund(fund_snapshot));
    }
}
