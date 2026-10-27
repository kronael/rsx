use crate::output::Failure;
use crate::symbols::find;
use crate::symbols::http_address;
use crate::symbols::to_raw;
use crate::symbols::Symbol;

fn symbol(symbol_id: u32) -> Symbol {
    Symbol {
        symbol_id,
        tick_size: 1,
        lot_size: 10,
        price_decimals: 2,
        qty_decimals: 3,
    }
}

#[test]
fn test_to_raw_scales_by_the_decimals() {
    assert_eq!(to_raw("12.34", 2), Ok(1234));
    assert_eq!(to_raw("12", 2), Ok(1200));
    assert_eq!(to_raw("0.5", 3), Ok(500));
    assert_eq!(to_raw(".5", 1), Ok(5));
    assert_eq!(to_raw("007", 0), Ok(7));
}

#[test]
fn test_to_raw_accepts_trailing_zeros_beyond_the_decimals() {
    assert_eq!(to_raw("1.2300", 2), Ok(123));
    assert_eq!(to_raw("5.0", 0), Ok(5));
}

#[test]
fn test_to_raw_rejects_digits_the_unit_cannot_hold() {
    let error = to_raw("1.234", 2).unwrap_err();
    assert!(error.contains("more than 2 decimals"), "{error}");
}

#[test]
fn test_to_raw_rejects_non_decimal_text() {
    for text in ["", ".", "-1", "+1", "1e3", "1.2.3", "abc", "1,5"] {
        assert!(to_raw(text, 2).is_err(), "{text:?}");
    }
}

#[test]
fn test_to_raw_rejects_overflow() {
    assert_eq!(to_raw("9223372036854775807", 0), Ok(i64::MAX));
    assert!(to_raw("9223372036854775808", 0).is_err());
    assert!(to_raw("92233720368547759", 2).is_err());
}

#[test]
fn test_default_min_notional_is_one_usd_from_the_decimals() {
    assert_eq!(symbol(1).default_min_notional(), 100_000);
}

#[test]
fn test_find_names_the_known_symbols_when_missing() {
    let symbols = [symbol(3), symbol(10)];
    assert_eq!(find(&symbols, 10).unwrap().symbol_id, 10);
    let failure = find(&symbols, 4).unwrap_err();
    assert!(matches!(failure, Failure::Misuse(_)), "{failure:?}");
    assert!(failure.message().contains("3, 10"), "{}", failure.message());
}

#[test]
fn test_http_address_is_the_websocket_host_and_port() {
    assert_eq!(
        http_address("ws://127.0.0.1:8088").unwrap(),
        "127.0.0.1:8088"
    );
    assert_eq!(http_address("ws://gw.local/path").unwrap(), "gw.local:80");
    assert!(matches!(
        http_address("wss://gw.local").unwrap_err(),
        Failure::Misuse(_)
    ));
}
