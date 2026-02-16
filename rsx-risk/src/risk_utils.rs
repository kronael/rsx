/// RISK.md §1. Fee with floor division.
/// Uses i128 intermediate to prevent overflow.
pub fn calculate_fee(
    qty: i64,
    price: i64,
    fee_bps: i64,
) -> i64 {
    let notional = qty as i128 * price as i128;
    let fee_128 = notional * fee_bps as i128;
    (fee_128.div_euclid(10_000)) as i64
}
