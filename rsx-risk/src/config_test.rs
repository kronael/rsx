use crate::config::checked_config_value;
use crate::config::checked_funding;
use crate::config::checked_funding_interval;
use crate::config::checked_margin_rules;
use crate::config::checked_schedule_time;
use crate::config::ConfigField;
use crate::config::SymbolRiskConfig;
use crate::margin::SymbolRiskParams;
use std::io::ErrorKind;

fn risk_config(initial: i64, maintenance: i64, max_leverage: i64) -> SymbolRiskConfig {
    SymbolRiskConfig {
        symbol_id: 10,
        config_version: 3,
        params: SymbolRiskParams {
            initial_margin_rate: initial,
            maintenance_margin_rate: maintenance,
            max_leverage,
            min_notional: 1,
        },
        taker_fee_bps: 5,
        maker_fee_bps: -1,
        tick_size: 1,
        funding_interval_secs: 28_800,
        funding_min_bps: -75,
        funding_max_bps: 75,
    }
}

#[test]
fn every_field_round_trips_through_its_column_name() {
    for field in ConfigField::ALL {
        assert_eq!(ConfigField::from_column(field.column()), Some(field));
    }
    assert_eq!(ConfigField::from_column("price_decimals"), None);
    assert_eq!(ConfigField::from_column("status"), None);
    assert_eq!(ConfigField::from_column("symbol_id"), None);
}

#[test]
fn quantities_must_be_positive_and_fees_may_be_negative() {
    assert!(checked_config_value(ConfigField::MaxLeverage, 0).is_err());
    assert!(checked_config_value(ConfigField::TickSize, -1).is_err());
    assert!(checked_config_value(ConfigField::MinNotional, 0).is_err());
    assert!(checked_config_value(ConfigField::MakerFeeBps, -1).is_ok());
    assert!(checked_config_value(ConfigField::FundingRateMinBps, -75).is_ok());
}

#[test]
fn int_columns_reject_a_value_outside_32_bits() {
    let error = checked_config_value(ConfigField::MaxLeverage, i64::from(i32::MAX) + 1)
        .expect_err("outside i32");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(error.to_string().contains("max_leverage"), "{error}");
    assert!(checked_config_value(ConfigField::MinNotional, i64::from(i32::MAX) + 1).is_ok());
}

#[test]
fn maintenance_must_be_below_initial_margin() {
    assert!(checked_margin_rules(&risk_config(500, 250, 20)).is_ok());
    let equal = checked_margin_rules(&risk_config(500, 500, 20)).expect_err("equal");
    assert_eq!(equal.kind(), ErrorKind::InvalidInput);
    assert!(checked_margin_rules(&risk_config(500, 600, 20)).is_err());
}

#[test]
fn funding_needs_a_positive_interval_and_ordered_bounds() {
    assert!(checked_funding(10, 3, 28_800, -75, 75).is_ok());
    assert!(checked_funding(10, 3, 28_800, 5, 5).is_ok());
    let zero = checked_funding(10, 3, 0, -75, 75).expect_err("zero interval");
    assert_eq!(zero.kind(), ErrorKind::InvalidData);
    assert!(zero.to_string().contains("funding_interval_sec"), "{zero}");
    let inverted = checked_funding(10, 3, 28_800, 76, 75).expect_err("inverted bounds");
    assert_eq!(inverted.kind(), ErrorKind::InvalidData);
    assert!(inverted.to_string().contains("v3"), "{inverted}");
}

#[test]
fn the_funding_interval_cannot_change_once_set() {
    assert!(checked_funding_interval(10, 0, 3_600).is_ok());
    assert!(checked_funding_interval(10, 28_800, 28_800).is_ok());
    let error = checked_funding_interval(10, 28_800, 3_600).expect_err("changed");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(error.to_string().contains("28800"), "{error}");
}

#[test]
fn leverage_below_one_is_rejected() {
    let error = checked_margin_rules(&risk_config(500, 250, 0)).expect_err("zero leverage");
    assert!(error.to_string().contains("max_leverage"), "{error}");
}

#[test]
fn a_version_is_never_dated_before_the_one_it_copies() {
    let now = 1_000;
    let in_an_hour = now + 3_600_000;
    let error = checked_schedule_time(10, 3, in_an_hour, now).expect_err("earlier than v3");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(error.to_string().contains("v3"), "{error}");
    assert!(
        error.to_string().contains(&in_an_hour.to_string()),
        "{error}"
    );
    assert!(checked_schedule_time(10, 3, in_an_hour, in_an_hour).is_ok());
    assert!(checked_schedule_time(10, 3, in_an_hour, in_an_hour + 1).is_ok());
}
