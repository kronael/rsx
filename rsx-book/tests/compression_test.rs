use rsx_book::compression::CompressionMap;

fn btc_map() -> CompressionMap {
    // BTC at $50,000, tick_size = 1 ($0.01)
    // mid_price = 5_000_000 ticks
    CompressionMap::new(5_000_000, 1)
}

#[test]
fn zone_0_1_to_1_resolution() {
    let m = btc_map();
    // Zone 0: 0-5% = 250,000 ticks from mid
    assert_eq!(m.compressions[0], 1);
    // Two adjacent ask prices should be adjacent indices
    let a = m.price_to_index(5_000_001);
    let b = m.price_to_index(5_000_002);
    assert_eq!(b - a, 1);
}

#[test]
fn zone_1_10_to_1() {
    let m = btc_map();
    assert_eq!(m.compressions[1], 10);
}

#[test]
fn zone_2_100_to_1() {
    let m = btc_map();
    assert_eq!(m.compressions[2], 100);
}

#[test]
fn zone_3_1000_to_1() {
    let m = btc_map();
    assert_eq!(m.compressions[3], 1000);
}

#[test]
fn zone_4_catchall_two_slots() {
    let m = btc_map();
    assert_eq!(m.zone_slots[4], 2);
}

#[test]
fn price_to_index_at_mid_price() {
    let m = btc_map();
    // Mid price should map to zone 0 ask side
    // (distance=0, side=ask since >= mid)
    let idx = m.price_to_index(5_000_000);
    // Should be in zone 0
    assert!(idx >= m.base_indices[0]);
    assert!(idx < m.base_indices[0] + m.zone_slots[0]);
}

#[test]
fn price_to_index_bid_side_decreasing() {
    let m = btc_map();
    // Bid prices below mid: farther from mid = smaller
    // index (bids stored in reverse)
    let close = m.price_to_index(4_999_999);
    let far = m.price_to_index(4_999_900);
    assert!(close > far);
}

#[test]
fn price_to_index_ask_side_increasing() {
    let m = btc_map();
    let close = m.price_to_index(5_000_001);
    let far = m.price_to_index(5_000_100);
    assert!(far > close);
}

#[test]
fn price_to_index_symmetric_around_mid() {
    let m = btc_map();
    let bid = m.price_to_index(4_999_990);
    let ask = m.price_to_index(5_000_010);
    // Both 10 ticks from mid, both in zone 0
    // Both should be valid indices
    assert!(bid < m.base_indices[0] + m.zone_slots[0]);
    assert!(ask < m.base_indices[0] + m.zone_slots[0]);
    assert_ne!(bid, ask);
}

#[test]
fn price_to_index_extreme_distance_catchall() {
    let m = btc_map();
    // Way beyond 50%
    let idx = m.price_to_index(10_000_000);
    assert!(
        idx >= m.base_indices[4]
            && idx < m.base_indices[4] + 2
    );
}

#[test]
fn total_slot_count_reasonable() {
    let m = btc_map();
    let total = m.total_slots();
    // Should be ~617K for BTC
    assert!(total > 500_000);
    assert!(total < 700_000);
}

#[test]
fn zone_boundary_0_1() {
    let m = btc_map();
    // Zone 0 edge: 5% of 5M = 250K ticks
    let in_z0 =
        m.price_to_index(5_000_000 + 249_999);
    let in_z1 =
        m.price_to_index(5_000_000 + 250_001);
    // z0 index should be in zone 0 range
    assert!(
        in_z0 >= m.base_indices[0]
            && in_z0
                < m.base_indices[0]
                    + m.zone_slots[0]
    );
    // z1 index should be in zone 1 range
    assert!(
        in_z1 >= m.base_indices[1]
            && in_z1
                < m.base_indices[1]
                    + m.zone_slots[1]
    );
}
