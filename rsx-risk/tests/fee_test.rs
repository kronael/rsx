use rsx_risk::risk_utils::calculate_fee;

#[test]
fn fee_floor_division_truncates() {
    // 7 * 3 * 10 / 10000 = 210/10000 = 0 (truncated)
    assert_eq!(calculate_fee(7, 3, 10), 0);
    // 100 * 100 * 10 / 10000 = 10
    assert_eq!(calculate_fee(100, 100, 10), 10);
}

#[test]
fn fee_negative_bps_is_rebate() {
    let fee = calculate_fee(100, 100, -5);
    // 100*100*(-5)/10000 = -5
    assert_eq!(fee, -5);
}

#[test]
fn fee_zero_qty_is_zero() {
    assert_eq!(calculate_fee(0, 100, 10), 0);
}

#[test]
fn fee_large_values_no_overflow() {
    // i128 intermediate prevents overflow
    let fee = calculate_fee(
        1_000_000_000,
        1_000_000_000,
        100,
    );
    // 1e9 * 1e9 * 100 / 10000 = 1e16
    assert_eq!(fee, 10_000_000_000_000_000);
}

#[test]
fn fee_one_lot_one_tick_one_bps() {
    // 1 * 1 * 1 / 10000 = 0
    assert_eq!(calculate_fee(1, 1, 1), 0);
}
