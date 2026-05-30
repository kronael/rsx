use rsx_risk::funding::calculate_payment;
use rsx_risk::funding::calculate_rate;
use rsx_risk::funding::interval_id;
use rsx_risk::funding::is_settlement_due;
use rsx_risk::funding::settle_symbol;
use rsx_risk::funding::FundingConfig;

fn default_config() -> FundingConfig {
    FundingConfig::default()
}

#[test]
fn funding_rate_mark_above_index_positive() {
    let cfg = default_config();
    let rate = calculate_rate(10100, 10000, &cfg);
    // (10100-10000)*10000/10000 = 100 bps
    assert_eq!(rate, 100);
}

#[test]
fn funding_rate_mark_below_index_negative() {
    let cfg = default_config();
    let rate = calculate_rate(9900, 10000, &cfg);
    assert_eq!(rate, -100);
}

#[test]
fn funding_rate_clamped_to_bounds() {
    let cfg = FundingConfig {
        rate_cap: 50,
        ..default_config()
    };
    let rate = calculate_rate(11000, 10000, &cfg);
    // 1000 bps unclamped, capped to 50
    assert_eq!(rate, 50);
    let rate2 = calculate_rate(9000, 10000, &cfg);
    assert_eq!(rate2, -50);
}

#[test]
fn funding_payment_long_pays_when_positive() {
    // long (net_qty>0), positive rate -> pays
    let p = calculate_payment(10, 1000, 100);
    // 10 * 1000 * 100 / 10000 = 100
    assert_eq!(p, 100);
    assert!(p > 0); // pays
}

#[test]
fn funding_payment_short_pays_when_negative() {
    // short (net_qty<0), negative rate -> pays
    // -10 * 1000 * (-100) / 10000 = 100 (pays)
    let p = calculate_payment(-10, 1000, -100);
    assert_eq!(p, 100);
    assert!(p > 0);
}

#[test]
fn funding_zero_position_no_payment() {
    let p = calculate_payment(0, 1000, 100);
    assert_eq!(p, 0);
}

#[test]
fn funding_rate_mark_equals_index_zero() {
    let cfg = default_config();
    let rate = calculate_rate(10000, 10000, &cfg);
    assert_eq!(rate, 0);
}

#[test]
fn funding_zero_sum_across_all_users() {
    let rate = 50; // 50 bps
    let mark = 1000;
    // User A: long 10, User B: short 10
    let pa = calculate_payment(10, mark, rate);
    let pb = calculate_payment(-10, mark, rate);
    assert_eq!(pa + pb, 0);
}

#[test]
fn funding_with_position_opened_mid_interval() {
    // Position opened mid-interval still gets full
    // funding at settlement. Funding applies to
    // current position at settlement time.
    let rate = 100;
    let mark = 1000;
    let p = calculate_payment(5, mark, rate);
    assert_eq!(p, 50);
}

#[test]
fn funding_extreme_divergence_clamped() {
    let cfg = FundingConfig {
        rate_cap: 100,
        ..default_config()
    };
    // mark=20000, index=10000 -> 10000 bps unclamped
    let rate = calculate_rate(20000, 10000, &cfg);
    assert_eq!(rate, 100);
}

#[test]
fn funding_settlement_idempotent() {
    let id1 = interval_id(28800, 28800);
    let id2 = interval_id(28800 + 100, 28800);
    // Same interval
    assert_eq!(id1, id2);
    assert!(!is_settlement_due(id1, 28800 + 100, 28800));
}

#[test]
fn funding_index_price_zero_handled() {
    let cfg = default_config();
    let rate = calculate_rate(10000, 0, &cfg);
    assert_eq!(rate, 0);
}

#[test]
fn funding_missed_interval_settled_on_startup() {
    // last_id=0, current=28800*3 -> 3 intervals passed
    assert!(is_settlement_due(0, 28800 * 3, 28800));
    assert!(is_settlement_due(1, 28800 * 3, 28800));
    assert!(is_settlement_due(2, 28800 * 3, 28800));
}

#[test]
fn funding_settlement_uses_latest_mark_price() {
    // Payment depends on mark price at settlement time.
    // Two settlements with different marks -> different
    // payments for the same position.
    let qty = 10;
    let rate = 50; // 50 bps
    let p1 = calculate_payment(qty, 1000, rate);
    let p2 = calculate_payment(qty, 2000, rate);
    assert_eq!(p1, 50); // 10*1000*50/10000
    assert_eq!(p2, 100); // 10*2000*50/10000
    assert_ne!(p1, p2);
}

#[test]
fn funding_payment_formula_qty_times_mark_times_rate() {
    // Explicit known-value formula verification.
    // qty=10, mark=1000, rate=50 bps
    // payment = 10 * 1000 * 50 / 10000 = 50
    let p = calculate_payment(10, 1000, 50);
    assert_eq!(p, 50);
}

#[test]
fn funding_payment_floors_toward_negative_infinity() {
    // Project rule: floor always. A short receiving funding has a
    // negative scaled value; -3 * 333 * 50 = -49950, /10000 = -4.995.
    // Floor = -5 (truncating `/` would wrongly give -4).
    let p = calculate_payment(-3, 333, 50);
    assert_eq!(p, -5);
    // The mirrored long pays floor(+4.995) = +4.
    let q = calculate_payment(3, 333, 50);
    assert_eq!(q, 4);
}

#[test]
fn funding_zero_sum_uneven_position_split() {
    // Invariant #9 (P0). Three longs of qty 1 and one short of qty 3.
    // Σ net_qty = 0. Independent per-user floor leaks -1 unit:
    //   long  +1 -> floor(0.1665e2/... ) per user, short -3 -> -4.995.
    // settle_symbol must net EXACTLY to zero.
    let net_qtys = [1, 1, 1, -3];
    let payments = settle_symbol(&net_qtys, 333, 50);
    let total: i64 = payments.iter().sum();
    assert_eq!(total, 0, "funding must be zero-sum (invariant #9)");
}

#[test]
fn funding_zero_sum_negative_rate_uneven_split() {
    // Same shape, negative rate (shorts pay longs). Must still net 0.
    let net_qtys = [1, 1, 1, -3];
    let payments = settle_symbol(&net_qtys, 333, -50);
    let total: i64 = payments.iter().sum();
    assert_eq!(total, 0);
}

#[test]
fn funding_zero_sum_many_users_pathological_marks() {
    // Stress: many users, marks/rates chosen to maximize remainders.
    // Σ net_qty held at 0; settle_symbol must net exactly to zero
    // regardless of how the floors fall.
    for &mark in &[1, 7, 333, 99_991, 1_000_003] {
        for &rate in &[1, 3, 17, 50, -1, -37, -100] {
            // 5 longs summing to +37, 4 shorts summing to -37.
            let net_qtys =
                [3, 7, 11, 5, 11, -13, -9, -8, -7];
            assert_eq!(net_qtys.iter().sum::<i64>(), 0);
            let payments =
                settle_symbol(&net_qtys, mark, rate);
            let total: i64 = payments.iter().sum();
            assert_eq!(
                total, 0,
                "zero-sum failed at mark={mark} rate={rate}"
            );
        }
    }
}

#[test]
fn funding_settle_matches_calculate_payment_when_exact() {
    // When every payment divides evenly, settle_symbol must equal
    // the per-user calculate_payment values (no spurious adjustment).
    let net_qtys = [10, -10];
    let payments = settle_symbol(&net_qtys, 1000, 50);
    assert_eq!(payments[0], calculate_payment(10, 1000, 50));
    assert_eq!(payments[1], calculate_payment(-10, 1000, 50));
    assert_eq!(payments.iter().sum::<i64>(), 0);
}

#[test]
fn funding_settle_empty_no_panic() {
    let payments = settle_symbol(&[], 1000, 50);
    assert!(payments.is_empty());
}

#[test]
fn funding_settle_residual_parked_on_largest_position() {
    // The short (qty 3, |3| largest) should absorb the residual so the
    // small longs keep their natural floored value.
    let net_qtys = [1, 1, 1, -3];
    let payments = settle_symbol(&net_qtys, 333, 50);
    // Each long floors to +0? 1*333*50=16650 -> floor(1.665)=1.
    assert_eq!(payments[0], 1);
    assert_eq!(payments[1], 1);
    assert_eq!(payments[2], 1);
    // Short carries -3 to balance the three +1 longs.
    assert_eq!(payments[3], -3);
    assert_eq!(payments.iter().sum::<i64>(), 0);
}
