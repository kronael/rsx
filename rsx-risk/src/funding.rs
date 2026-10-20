use crate::persist::FundingRecord;
use crate::persist::PersistEvent;
use crate::shard::RiskShard;

/// RISK.md §5.
#[derive(Clone, Debug)]
pub struct FundingConfig {
    pub interval_secs: u64,
    pub rate_cap: i64,
}

impl Default for FundingConfig {
    fn default() -> Self {
        Self {
            interval_secs: 28800,
            rate_cap: 100, // 100 bps = 1%
        }
    }
}

/// RISK.md §5. Rate in bps.
pub fn calculate_rate(mark: i64, index: i64, config: &FundingConfig) -> i64 {
    if index == 0 {
        return 0;
    }
    let premium_raw = (mark as i128 - index as i128) * 10_000 / index as i128;
    let premium = premium_raw.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    premium.clamp(-config.rate_cap, config.rate_cap)
}

/// RISK.md §5. Payment = floor(net_qty * mark * rate / 10_000).
/// Positive = user pays, negative = user receives.
///
/// Rounds toward negative infinity (project rule: "floor always").
/// Rust's `/` truncates toward zero, so a negative scaled value with a
/// nonzero remainder would round the wrong way; `div_euclid` floors.
pub fn calculate_payment(net_qty: i64, mark: i64, rate: i64) -> i64 {
    let scaled = net_qty as i128 * mark as i128 * rate as i128;
    floor_div_10k(scaled)
}

/// floor(x / 10_000), then clamp into i64. `div_euclid` gives the
/// floored quotient for negative `x` (truncating `/` does not).
fn floor_div_10k(x: i128) -> i64 {
    let q = x.div_euclid(10_000);
    q.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// RISK.md §5, invariant #9 (funding zero-sum per symbol).
/// Compute each user's funding payment for one symbol.
/// `net_qtys[i]` is the signed position for user i; returns
/// payments in the same order, positive = pays.
///
/// A Risk shard owns only a subset of users, so its local
/// `Σ net_qty` is generally non-zero — the offsetting users
/// live on other shards. We therefore do NOT force this
/// shard's column to zero (that would erase legitimate
/// one-sided funding). What we DO eliminate is the
/// integer-division leak *within this shard*: summing each
/// user's individually-truncated payment differs from the
/// single truncation of the shard's aggregate net_qty by a
/// small residual. That residual is value created/destroyed
/// by rounding alone; we park it on the largest-magnitude
/// position. Globally (Σ over shards) `Σ net_qty == 0`
/// (invariant #4), so the aggregate-based payments cancel.
pub fn settle_symbol(net_qtys: &[i64], mark: i64, rate: i64) -> Vec<i64> {
    let mut payments: Vec<i64> = net_qtys
        .iter()
        .map(|&q| calculate_payment(q, mark, rate))
        .collect();
    let sum_rounded: i128 = payments.iter().map(|&p| p as i128).sum();
    let net_total: i128 = net_qtys.iter().map(|&q| q as i128).sum();
    let agg_payment = calculate_payment_i128(net_total, mark, rate);
    let residual = (sum_rounded - agg_payment).clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    if residual != 0 {
        if let Some(i) = largest_idx(net_qtys) {
            payments[i] = payments[i].saturating_sub(residual);
        }
    }
    payments
}

/// floor(net_qty * mark * rate / 10_000) at i128 width, clamped into
/// i128's i64 range. The aggregate truncation that per-user payments
/// are reconciled against in `settle_symbol`.
fn calculate_payment_i128(net_qty: i128, mark: i64, rate: i64) -> i128 {
    (net_qty * mark as i128 * rate as i128)
        .div_euclid(10_000)
        .clamp(i64::MIN as i128, i64::MAX as i128)
}

/// Index of the largest-|net_qty| element (first on ties). `None` if
/// the slice is empty. The residual is parked here because adjusting a
/// few units against the biggest position is the least material.
fn largest_idx(net_qtys: &[i64]) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (i, &q) in net_qtys.iter().enumerate() {
        let mag = q.saturating_abs();
        match best {
            Some((_, b)) if mag <= b => {}
            _ => best = Some((i, mag)),
        }
    }
    best.map(|(i, _)| i)
}

pub fn interval_id(unix_secs: u64, interval_secs: u64) -> u64 {
    if interval_secs == 0 {
        return 0;
    }
    unix_secs / interval_secs
}

pub fn is_settlement_due(last_id: u64, current_secs: u64, interval_secs: u64) -> bool {
    interval_id(current_secs, interval_secs) > last_id
}

impl RiskShard {
    /// RISK.md §5. Settle funding if interval elapsed.
    ///
    /// Invariant #9 (funding zero-sum per symbol per interval): for
    /// each symbol we compute one `(rate, mark)` and call
    /// `settle_symbol` over every exposed user's signed
    /// `net_qty` in one shot. `settle_symbol` removes the intra-shard
    /// integer-division residual (parks it on the largest position)
    /// without forcing this shard's one-sided column to zero — the
    /// offsetting users live on other shards, and globally
    /// `Σ net_qty == 0` (invariant #4), so the aggregate payments
    /// cancel across shards.
    pub fn maybe_settle_funding(&mut self, now_secs: u64) {
        if !is_settlement_due(
            self.last_funding_id,
            now_secs,
            self.funding_config.interval_secs,
        ) {
            return;
        }

        let new_id = interval_id(now_secs, self.funding_config.interval_secs);

        for sid in 0..self.max_symbols {
            let mark = self.mark_prices[sid];
            let index = self.index_prices[sid].price;
            if mark == 0 || index == 0 {
                continue;
            }
            let rate = calculate_rate(mark, index, &self.funding_config);
            // Funding settlement is periodic (not per-order hot
            // path). Collect exposed users' signed net_qty into a
            // slice with a parallel user_id list (alignment must be
            // exact), settle the whole symbol in one zero-sum pass,
            // then distribute payments back. Vec avoids a borrow
            // conflict with the &mut self calls below.
            let users: Vec<u32> = self.exposure.users_for_symbol(sid).to_vec();
            let mut settle_users: Vec<u32> = Vec::with_capacity(users.len());
            let mut net_qtys: Vec<i64> = Vec::with_capacity(users.len());
            for user_id in users {
                if let Some(pos) = self.positions.get(&(user_id, sid as u32)) {
                    settle_users.push(user_id);
                    net_qtys.push(pos.net_qty());
                }
            }
            let payments = settle_symbol(&net_qtys, mark, rate);
            for (user_id, payment) in settle_users.iter().zip(payments) {
                let user_id = *user_id;
                if let Some(acct) = self.accounts.get_mut(&user_id) {
                    acct.deduct_fee(payment);
                }
                let acct_clone = self.accounts.get(&user_id).cloned();
                self.push_persist(PersistEvent::Funding(FundingRecord {
                    user_id,
                    symbol_id: sid as u32,
                    amount: payment,
                    rate,
                    settlement_ts: now_secs,
                }));
                if let Some(acct) = acct_clone {
                    self.push_persist(PersistEvent::Account(acct));
                }
            }
        }

        self.last_funding_id = new_id;
    }
}
