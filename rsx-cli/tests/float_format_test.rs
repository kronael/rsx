//! Float formatting must work in a debug build, which compiles with the
//! cranelift backend. serde_json formats floats through `zmij`, whose SSE2
//! path uses `_mm_mulhi_epu16`; cranelift cannot lower it, so an unlisted
//! `zmij` makes each of these abort. See `.cargo/config.toml`.

use serde_json::json;

#[test]
fn serde_json_formats_f64() {
    assert_eq!(serde_json::to_string(&1.5f64).unwrap(), "1.5");
    assert_eq!(serde_json::to_string(&0.049f64).unwrap(), "0.049");
    assert_eq!(serde_json::to_string(&123456.789f64).unwrap(), "123456.789");
}

#[test]
fn serde_json_formats_f32() {
    assert_eq!(serde_json::to_string(&1.5f32).unwrap(), "1.5");
    assert_eq!(serde_json::to_string(&0.1f32).unwrap(), "0.1");
}

#[test]
fn serde_json_value_number_prints() {
    let value = json!({"rate": 2.25});
    assert_eq!(value.to_string(), r#"{"rate":2.25}"#);
}

#[test]
fn core_formats_floats() {
    assert_eq!(format!("{}", 0.1f64 + 0.2f64), "0.30000000000000004");
    assert_eq!(format!("{:.3}", 2.0f64 / 3.0), "0.667");
    assert_eq!(format!("{:e}", 1500.0f64), "1.5e3");
}
