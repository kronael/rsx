use crate::config::checked_config_value;
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
