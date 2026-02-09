use rsx_risk::price::calculate_index;
use rsx_risk::price::IndexPrice;
use rsx_risk::types::BboUpdate;

#[test]
fn index_price_size_weighted_mid() {
    // bid=100 qty=10, ask=110 qty=20
    // index = (100*20 + 110*10) / 30 = 3100/30 = 103
    let p = calculate_index(100, 10, 110, 20, 0);
    assert_eq!(p, 103);
}

#[test]
fn index_price_balanced_book_equals_mid() {
    // bid=100 qty=10, ask=110 qty=10
    // index = (100*10 + 110*10) / 20 = 2100/20 = 105
    let p = calculate_index(100, 10, 110, 10, 0);
    assert_eq!(p, 105);
}

#[test]
fn index_price_imbalanced_favors_thicker_side() {
    // Heavy ask side -> index closer to bid
    let p = calculate_index(100, 1, 110, 100, 0);
    // (100*100 + 110*1) / 101 = 10110/101 = 100
    assert_eq!(p, 100);
}

#[test]
fn index_price_one_side_zero_qty_uses_that_side() {
    // bid_qty=0: use ask_px
    assert_eq!(calculate_index(100, 0, 110, 10, 0), 110);
    // ask_qty=0: use bid_px
    assert_eq!(calculate_index(100, 10, 110, 0, 0), 100);
}

#[test]
fn index_price_both_sides_zero_qty_keeps_last() {
    let p = calculate_index(100, 0, 110, 0, 99);
    assert_eq!(p, 99);
}

#[test]
fn index_price_no_bbo_ever_uses_mark_price() {
    let mut ip = IndexPrice::default();
    assert!(!ip.valid);
    // Without any BBO, price stays 0 (caller uses
    // mark price as fallback when valid==false)
    assert_eq!(ip.price, 0);

    // After BBO, valid becomes true
    ip.update_from_bbo(&BboUpdate {
        symbol_id: 0,
        bid_px: 100,
        bid_qty: 10,
        ask_px: 110,
        ask_qty: 10,
    });
    assert!(ip.valid);
}

#[test]
fn index_price_max_values_no_overflow() {
    // Uses i128 internally
    let p = calculate_index(
        1_000_000_000,
        1_000_000_000,
        1_000_000_001,
        1_000_000_000,
        0,
    );
    // Should be ~mid without overflow
    assert!(p > 0);
}

#[test]
fn index_price_spread_zero_equals_price() {
    let p = calculate_index(100, 10, 100, 10, 0);
    assert_eq!(p, 100);
}
