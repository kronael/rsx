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
    /// debug builds or wrap in release.
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
    /// specs/2/13-liquidator.md. Moves the account's deficit, `-equity`
    /// over its whole portfolio, from the symbol's insurance fund to
    /// the account's collateral. An account at or above zero equity
    /// moves nothing and writes nothing.
    pub(crate) fn process_socialized_loss(&mut self, loss: &SocializedLoss, now_ns: u64) {
        let Some(account) = self.accounts.get(&loss.user_id) else {
            return;
        };
        let equity = self
            .margin
            .calculate(
                account,
                self.iter_positions_for_user(loss.user_id),
                &self.fallback_mark_prices,
                self.frozen_for_user(loss.user_id),
            )
            .equity;
        if equity >= 0 {
            return;
        }
        let deficit = equity.saturating_neg();

        let fund = self
            .insurance_funds
            .entry(loss.symbol_id)
            .or_insert_with(|| InsuranceFund::new(loss.symbol_id, 0));
        fund.deduct(deficit);
        let fund_snapshot = fund.clone();

        let Some(account) = self.accounts.get_mut(&loss.user_id) else {
            return;
        };
        account.credit(deficit);
        let account_snapshot = account.clone();

        warn!(
            "socialized loss: user={} symbol={} \
             round={} qty={} price={} deficit={}",
            loss.user_id, loss.symbol_id, loss.round, loss.qty, loss.price, deficit
        );

        self.push_persist(PersistEvent::Account(account_snapshot));
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
