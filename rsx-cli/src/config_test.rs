use crate::config::parse_setting;
use crate::config::scheduled_json;
use crate::config::to_failure;
use crate::config::SymbolView;
use crate::output::Failure;
use rsx_risk::config::ConfigField;
use rsx_risk::config::ScheduledVersion;
use serde_json::json;
use serde_json::Value;
use std::io::Error;
use std::io::ErrorKind;

fn row(version: i64, effective_at_ms: i64) -> Value {
    json!({
        "symbol_id": 10,
        "config_version": version,
        "effective_at_ms": effective_at_ms,
        "tick_size": 1,
        "lot_size": 100_000,
        "initial_margin_rate_bps": 1000,
        "maintenance_margin_rate_bps": 500,
        "max_leverage": 10,
        "taker_fee_bps": 5,
        "maker_fee_bps": null,
        "min_notional": 10_000_000_000i64,
    })
}

fn view() -> SymbolView {
    SymbolView {
        symbol_id: 10,
        name: Some("PENGU".to_string()),
        scheduled: vec![row(1, 0), row(2, 100), row(3, 10_000)],
        applied: Some(json!({"symbol_id": 10, "config_version": 2})),
    }
}

#[test]
fn a_setting_is_a_column_and_a_whole_number() {
    assert_eq!(
        parse_setting("max_leverage=5"),
        Ok((ConfigField::MaxLeverage, 5))
    );
    assert_eq!(
        parse_setting("maker_fee_bps=-2"),
        Ok((ConfigField::MakerFeeBps, -2))
    );
}

#[test]
fn a_setting_outside_the_closed_list_names_the_settings() {
    for text in [
        "max_leverage",
        "status=halted",
        "price_decimals=3",
        "max_leverage=x",
    ] {
        assert!(parse_setting(text).is_err(), "{text}");
    }
    let error = parse_setting("status=halted").unwrap_err();
    assert!(error.contains("tick_size"), "{error}");
}

#[test]
fn the_active_version_is_the_highest_one_already_effective() {
    assert_eq!(view().active_version(500), Some(2));
    assert_eq!(view().active_version(10_000), Some(3));
    assert_eq!(view().active_version(-1), None);
}

#[test]
fn symbol_json_carries_rows_with_their_columns() {
    let value = view().to_json(500);
    assert_eq!(value["symbol_id"], 10);
    assert_eq!(value["name"], "PENGU");
    assert_eq!(value["active_version"], 2);
    assert_eq!(value["scheduled"].as_array().unwrap().len(), 3);
    assert_eq!(value["scheduled"][2]["config_version"], 3);
    assert_eq!(value["applied"]["config_version"], 2);
}

#[test]
fn a_symbol_never_applied_has_a_null_applied_row() {
    let view = SymbolView {
        applied: None,
        name: None,
        ..view()
    };
    let value = view.to_json(500);
    assert!(value["applied"].is_null());
    assert!(value["name"].is_null());
}

#[test]
fn symbol_text_has_a_header_and_a_row_per_version() {
    let text = view().to_text(500);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5, "{text}");
    assert_eq!(lines[0], "symbol 10 PENGU  active v2  applied v2");
    assert!(lines[1].contains("max_leverage"), "{text}");
    assert!(lines[3].contains("10000000000"), "{text}");
    assert!(lines[2].contains(" -  "), "a null column prints -: {text}");
}

#[test]
fn scheduled_json_has_the_three_keys() {
    let value = scheduled_json(&ScheduledVersion {
        symbol_id: 10,
        config_version: 3,
        effective_at_ms: 99,
    });
    assert_eq!(
        value,
        json!({"symbol_id": 10, "config_version": 3, "effective_at_ms": 99})
    );
}

#[test]
fn a_rejected_change_is_misuse_and_the_rest_is_runtime() {
    let rejected = to_failure(Error::new(ErrorKind::InvalidInput, "max_leverage is 0"));
    assert!(matches!(rejected, Failure::Misuse(_)), "{rejected:?}");
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::AlreadyExists,
        ErrorKind::Other,
    ] {
        let failure = to_failure(Error::new(kind, "x"));
        assert!(matches!(failure, Failure::Runtime(_)), "{failure:?}");
    }
}
