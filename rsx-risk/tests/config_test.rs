use rsx_risk::config::load_shard_config;

/// FIX 4: max_slip_bps > 10_000 would drive the long-side
/// liquidation price negative (mark*(10000-slip)/10000). The
/// loader must fail fast at startup instead of generating bad
/// orders later.
///
/// Both bounds live in one test: they mutate the same process
/// env var, so running them as separate parallel tests races.
#[test]
fn max_slip_bps_bounds_enforced_at_load() {
    // Over the inclusive upper bound: must be rejected.
    std::env::set_var("RSX_LIQUIDATION_MAX_SLIP_BPS", "10001");
    let over = load_shard_config();
    assert!(
        over.is_err(),
        "max_slip_bps > 10000 must be rejected at load"
    );

    // Exactly at the inclusive upper bound: accepted.
    std::env::set_var("RSX_LIQUIDATION_MAX_SLIP_BPS", "10000");
    let at = load_shard_config();
    std::env::remove_var("RSX_LIQUIDATION_MAX_SLIP_BPS");
    let cfg = at.expect("10000 is the inclusive upper bound");
    assert_eq!(cfg.liquidation_config.max_slip_bps, 10_000);
}

#[test]
fn min_notional_must_be_present_and_positive() {
    use rsx_risk::config::checked_min_notional;
    assert_eq!(checked_min_notional(3, 2, Some(1)).unwrap(), 1);
    assert_eq!(
        checked_min_notional(3, 2, Some(i64::MAX)).unwrap(),
        i64::MAX
    );
    for bad in [Some(0), Some(-1), Some(i64::MIN)] {
        let err = checked_min_notional(3, 2, bad).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let text = err.to_string();
        assert!(text.contains("symbol 3 v2"), "{text}");
        assert!(text.contains("min_notional"), "{text}");
        assert!(text.contains("> 0"), "{text}");
    }
    let err = checked_min_notional(3, 2, None).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "symbol 3 v2: min_notional is NULL");
}

#[test]
fn symbol_decimals_must_sum_to_the_notional_unit() {
    use rsx_risk::config::checked_notional_decimals;
    use rsx_risk::config::NOTIONAL_DECIMALS;
    assert_eq!(NOTIONAL_DECIMALS, 10);
    // Every preset split.
    for (price_decimals, qty_decimals) in [(2, 8), (4, 6), (6, 4), (0, 10), (10, 0)] {
        checked_notional_decimals(1, 1, price_decimals, qty_decimals).unwrap();
    }
    for (price_decimals, qty_decimals) in [(2, 7), (6, 5), (0, 0), (-1, 12), (i16::MAX, i16::MAX)] {
        let err = checked_notional_decimals(7, 3, price_decimals, qty_decimals).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let text = err.to_string();
        assert!(text.starts_with("symbol 7 v3: price_decimals"), "{text}");
        assert!(text.ends_with("needs 10"), "{text}");
    }
}
