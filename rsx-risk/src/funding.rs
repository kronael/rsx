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
    let premium =
        (mark - index) * 10_000 / index;
    premium.clamp(-config.rate_cap, config.rate_cap)
}

/// RISK.md §5. Payment = net_qty * mark * rate / 10_000.
/// Positive = user pays, negative = user receives.
pub fn calculate_payment(
    net_qty: i64,
    mark: i64,
    rate: i64,
) -> i64 {
    // Use i128 to prevent overflow
    (net_qty as i128 * mark as i128 * rate as i128
        / 10_000) as i64
}

pub fn interval_id(
    unix_secs: u64,
    interval_secs: u64,
) -> u64 {
    unix_secs / interval_secs
}

pub fn is_settlement_due(
    last_id: u64,
    current_secs: u64,
    interval_secs: u64,
) -> bool {
    interval_id(current_secs, interval_secs) > last_id
}
