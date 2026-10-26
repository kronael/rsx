use crate::persist::FundingRecord;
use crate::persist::FundingSettlement;
use crate::persist::PersistEvent;
use crate::shard::RiskShard;
use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::info;
use tracing::warn;

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

/// Last settled funding interval per symbol for `shard_id`, indexed by
/// symbol id; `None` for a symbol that has settled none.
pub async fn load_funding_settled(
    client: &Client,
    shard_id: u32,
    max_symbols: usize,
) -> Result<Vec<Option<u64>>, Error> {
    let mut settled = vec![None; max_symbols];
    let rows = client
        .query(
            "SELECT symbol_id, max(interval_id) \
             FROM funding_intervals \
             WHERE shard_id = $1 \
             GROUP BY symbol_id",
            &[&(shard_id as i32)],
        )
        .await?;
    for row in &rows {
        let symbol_id: i32 = row.get(0);
        let interval_id: i64 = row.get(1);
        if let Some(slot) = settled.get_mut(symbol_id as usize) {
            *slot = Some(interval_id as u64);
        }
    }
    Ok(settled)
}

impl RiskShard {
    /// RISK.md §5. Settle every funding interval each configured
    /// symbol has not settled yet, oldest first, one persist event per
    /// (symbol, interval).
    ///
    /// A symbol with no recorded interval settles the current one. A
    /// missed interval is settled at the latest mark and index, so a
    /// catch-up after downtime charges the current premium for each
    /// interval it covers. A symbol with open positions and no mark or
    /// index price waits, and is retried on the next second.
    ///
    /// Invariant #9 (funding zero-sum per symbol per interval): each
    /// interval's payments come from one `settle_symbol` call over
    /// every exposed user's signed `net_qty`, which removes the
    /// intra-shard integer-division residual without forcing this
    /// shard's one-sided column to zero. The offsetting users live on
    /// other shards, and globally `Σ net_qty == 0` (invariant #4), so
    /// the aggregate payments cancel across shards.
    pub fn maybe_settle_funding(&mut self, now_secs: u64) {
        if now_secs == self.funding_checked_secs {
            return;
        }
        self.funding_checked_secs = now_secs;
        let interval_secs = self.funding_config.interval_secs;
        let current_id = interval_id(now_secs, interval_secs);
        let mut waiting = false;
        for sid in 0..self.max_symbols {
            let Some(Some(_)) = self.margin.symbol_params.get(sid) else {
                continue;
            };
            let settled = self.funding_settled[sid];
            if let Some(last_id) = settled {
                if !is_settlement_due(last_id, now_secs, interval_secs) {
                    continue;
                }
            }
            let first_id = settled.map_or(current_id, |last_id| last_id + 1);
            for id in first_id..=current_id {
                if !self.try_settle_interval(sid, id, now_secs) {
                    waiting = true;
                    break;
                }
            }
        }
        if waiting && !self.funding_stalled {
            warn!("funding settlement waiting for mark and index prices");
        }
        if !waiting && self.funding_stalled {
            info!("funding settlement resumed");
        }
        self.funding_stalled = waiting;
    }

    /// Settle one interval for one symbol. `false` when open positions
    /// have no mark or index price yet; nothing is changed then.
    fn try_settle_interval(&mut self, sid: usize, id: u64, now_secs: u64) -> bool {
        let mut exposed: Vec<(u32, i64)> = Vec::new();
        for &user_id in self.exposure.users_for_symbol(sid) {
            if let Some(position) = self.positions.get(&(user_id, sid as u32)) {
                exposed.push((user_id, position.net_qty()));
            }
        }
        exposed.sort_unstable_by_key(|&(user_id, _)| user_id);

        let mark = self.mark_prices[sid];
        let index = self.index_prices[sid].price;
        let is_priced = mark != 0 && index != 0;
        let has_exposure = exposed.iter().any(|&(_, net_qty)| net_qty != 0);
        if has_exposure && !is_priced {
            return false;
        }
        let rate = if is_priced {
            calculate_rate(mark, index, &self.funding_config)
        } else {
            0
        };

        let net_qtys: Vec<i64> = exposed.iter().map(|&(_, net_qty)| net_qty).collect();
        let amounts = settle_symbol(&net_qtys, mark, rate);
        let mut payments = Vec::with_capacity(exposed.len());
        let mut accounts = Vec::with_capacity(exposed.len());
        for (&(user_id, _), amount) in exposed.iter().zip(amounts) {
            self.ensure_account(user_id);
            if let Some(account) = self.accounts.get_mut(&user_id) {
                account.deduct_fee(amount);
                accounts.push(account.clone());
            }
            payments.push(FundingRecord {
                user_id,
                symbol_id: sid as u32,
                interval_id: id,
                amount,
                rate,
                settlement_ts: now_secs,
            });
        }
        self.push_persist(PersistEvent::FundingSettlement(FundingSettlement {
            symbol_id: sid as u32,
            interval_id: id,
            payments,
            accounts,
        }));
        self.funding_settled[sid] = Some(id);
        true
    }

    /// Apply the funding another main settled since this shard loaded
    /// its state: charge the recorded payments to the in-memory
    /// accounts and advance the settled intervals. A warm replica runs
    /// this once it holds the advisory lock, before its first tick, so
    /// it never settles an interval the previous main already did and
    /// never loses that main's collateral changes.
    pub async fn follow_funding(&mut self, client: &Client) -> Result<(), Error> {
        client
            .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let followed = self.read_followed_funding(client).await;
        let end = if followed.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        client.batch_execute(end).await?;
        let (settled, payments) = followed?;
        for (user_id, amount) in payments {
            self.ensure_account(user_id);
            if let Some(account) = self.accounts.get_mut(&user_id) {
                account.deduct_fee(amount);
            }
        }
        for (sid, interval) in settled.into_iter().enumerate() {
            if interval.is_some() {
                self.funding_settled[sid] = interval;
            }
        }
        Ok(())
    }

    async fn read_followed_funding(
        &self,
        client: &Client,
    ) -> Result<(Vec<Option<u64>>, Vec<(u32, i64)>), Error> {
        let persisted = load_funding_settled(client, self.shard_id, self.max_symbols).await?;
        let mut settled = vec![None; self.max_symbols];
        let mut payments = Vec::new();
        for sid in 0..self.max_symbols {
            let Some(latest) = persisted[sid] else {
                continue;
            };
            let applied = self.funding_settled[sid];
            if applied.is_some_and(|applied_id| applied_id >= latest) {
                continue;
            }
            let after = applied.map_or(-1, |applied_id| applied_id as i64);
            let rows = client
                .query(
                    "SELECT user_id, amount FROM funding \
                     WHERE symbol_id = $1 \
                       AND interval_id > $2 AND interval_id <= $3 \
                       AND user_id % $4 = $5",
                    &[
                        &(sid as i32),
                        &after,
                        &(latest as i64),
                        &(self.shard_count as i32),
                        &(self.shard_id as i32),
                    ],
                )
                .await?;
            for row in &rows {
                let user_id: i32 = row.get(0);
                payments.push((user_id as u32, row.get::<_, i64>(1)));
            }
            settled[sid] = Some(latest);
        }
        Ok((settled, payments))
    }
}
