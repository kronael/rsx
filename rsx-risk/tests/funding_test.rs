use rsx_risk::funding::calculate_payment;
use rsx_risk::funding::calculate_rate;
use rsx_risk::funding::interval_id;
use rsx_risk::funding::is_settlement_due;
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
