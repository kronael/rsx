/// Fee on `qty * price` at `fee_bps` basis points, floor division; a
/// negative `fee_bps` is a rebate. Saturates toward the correct sign.
pub fn calculate_fee(qty: i64, price: i64, fee_bps: i64) -> i64 {
    let notional = qty as i128 * price as i128;
    let fee_128 = notional * fee_bps as i128;
    let result = fee_128.div_euclid(10_000);
    i64::try_from(result).unwrap_or(if result < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
#[path = "fee_utils_test.rs"]
mod fee_utils_test;
