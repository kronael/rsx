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
