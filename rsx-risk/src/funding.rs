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
pub fn calculate_rate(
    mark: i64,
    index: i64,
    config: &FundingConfig,
) -> i64 {
    if index == 0 {
        return 0;
    }
    let premium_raw = (mark as i128 - index as i128)
        * 10_000
        / index as i128;
    let premium = premium_raw
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    premium.clamp(-config.rate_cap, config.rate_cap)
}

/// RISK.md §5. Payment = floor(net_qty * mark * rate / 10_000).
/// Positive = user pays, negative = user receives.
///
/// Rounds toward negative infinity (project rule: "floor always").
/// Rust's `/` truncates toward zero, so a negative scaled value with a
/// nonzero remainder would round the wrong way; `div_euclid` floors.
pub fn calculate_payment(
    net_qty: i64,
    mark: i64,
    rate: i64,
) -> i64 {
    let scaled = net_qty as i128 * mark as i128 * rate as i128;
    floor_div_10k(scaled)
}

/// floor(x / 10_000), then clamp into i64. `div_euclid` gives the
/// floored quotient for negative `x` (truncating `/` does not).
fn floor_div_10k(x: i128) -> i64 {
    let q = x.div_euclid(10_000);
    q.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// RISK.md §5 + invariant #9 (funding zero-sum per symbol per interval).
///
/// Computes one funding payment per user for a single symbol and
/// returns them in the same order as `net_qtys`. The sum of the
/// returned payments is **exactly zero** whenever `Σ net_qtys == 0`
/// (which invariant #4 guarantees: every fill adds +qty to one side
/// and -qty to the other).
///
/// Per-user floor rounding alone does NOT net to zero — e.g. three
/// longs of qty 1 and one short of qty 3 each floor independently and
/// can leak ±1 unit per interval. We round every payment with `floor`
/// (project rule) and then push the entire rounding residual onto the
/// largest-magnitude position, where a sub-unit adjustment is
/// immaterial. This makes Σ payments == 0 by construction.
pub fn settle_symbol(
    net_qtys: &[i64],
    mark: i64,
    rate: i64,
) -> Vec<i64> {
    let mut payments: Vec<i64> = net_qtys
        .iter()
        .map(|&q| calculate_payment(q, mark, rate))
        .collect();

    // Residual = Σ payments (zero only by luck after independent
    // flooring). Absorb it on the largest-magnitude position so the
    // book nets exactly to zero.
    let residual: i128 =
        payments.iter().map(|&p| p as i128).sum();
    if residual != 0 {
        if let Some(idx) = largest_abs_idx(net_qtys) {
            let adj = (payments[idx] as i128 - residual)
                .clamp(i64::MIN as i128, i64::MAX as i128)
                as i64;
            payments[idx] = adj;
        }
    }
    payments
}

/// Index of the largest-|net_qty| element (first on ties). `None` if
/// the slice is empty. The residual is parked here because adjusting a
/// few units against the biggest position is the least material.
fn largest_abs_idx(net_qtys: &[i64]) -> Option<usize> {
    net_qtys
        .iter()
        .enumerate()
        .max_by_key(|(_, &q)| (q as i128).unsigned_abs())
        .map(|(i, _)| i)
}

pub fn interval_id(
    unix_secs: u64,
    interval_secs: u64,
) -> u64 {
    if interval_secs == 0 {
        return 0;
    }
    unix_secs / interval_secs
}

pub fn is_settlement_due(
    last_id: u64,
    current_secs: u64,
    interval_secs: u64,
) -> bool {
    interval_id(current_secs, interval_secs) > last_id
}
