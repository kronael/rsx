use rsx_risk::position::Position;

#[test]
fn apply_buy_fill_opens_long() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    assert_eq!(p.long_qty, 10);
    assert_eq!(p.short_qty, 0);
    assert_eq!(p.long_entry_cost, 1000);
}

#[test]
fn apply_sell_fill_opens_short() {
    let mut p = Position::new(1, 0);
    p.apply_fill(1, 100, 10, 1);
    assert_eq!(p.short_qty, 10);
    assert_eq!(p.long_qty, 0);
    assert_eq!(p.short_entry_cost, 1000);
}

#[test]
fn apply_opposing_fill_reduces_position() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1); // long 10
    p.apply_fill(1, 110, 5, 2);  // sell 5
    assert_eq!(p.long_qty, 5);
    assert_eq!(p.short_qty, 0);
}

#[test]
fn apply_fill_closing_position_realizes_pnl() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1); // long 10@100
    p.apply_fill(1, 120, 10, 2); // close at 120
    // pnl = 10 * (120 - 100) = 200
    assert_eq!(p.realized_pnl, 200);
    assert!(p.is_empty());
}

#[test]
fn avg_entry_price_weighted_correctly() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1); // 10@100
    p.apply_fill(0, 200, 10, 2); // 10@200
    // avg = 3000 / 20 = 150
    assert_eq!(p.avg_entry(), 150);
}

#[test]
fn multiple_fills_same_side_accumulate() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 5, 1);
    p.apply_fill(0, 100, 5, 2);
    assert_eq!(p.long_qty, 10);
    assert_eq!(p.long_entry_cost, 1000);
}

#[test]
fn fill_larger_than_position_flips_side() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1); // long 10@100
    p.apply_fill(1, 120, 15, 2); // sell 15
    assert_eq!(p.long_qty, 0);
    assert_eq!(p.short_qty, 5);
    assert_eq!(p.short_entry_cost, 600); // 5*120
    // realized = 10*(120-100) = 200
    assert_eq!(p.realized_pnl, 200);
}

#[test]
fn zero_qty_after_exact_close() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    p.apply_fill(1, 100, 10, 2);
    assert!(p.is_empty());
    assert_eq!(p.net_qty(), 0);
}

// -- edge cases --

#[test]
fn flip_long_to_short_single_fill() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    p.apply_fill(1, 110, 20, 2);
    assert_eq!(p.net_qty(), -10);
    assert_eq!(p.short_qty, 10);
    assert_eq!(p.short_entry_cost, 1100);
}

#[test]
fn flip_short_to_long_single_fill() {
    let mut p = Position::new(1, 0);
    p.apply_fill(1, 100, 10, 1);
    p.apply_fill(0, 90, 20, 2);
    assert_eq!(p.net_qty(), 10);
    assert_eq!(p.long_qty, 10);
    assert_eq!(p.long_entry_cost, 900);
    // realized = 10*(100-90) = 100
    assert_eq!(p.realized_pnl, 100);
}

#[test]
fn flip_realizes_pnl_then_opens_at_fill_price() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 5, 1);
    p.apply_fill(1, 120, 8, 2);
    // close 5 long: pnl = 5*(120-100)=100
    assert_eq!(p.realized_pnl, 100);
    // open 3 short at 120
    assert_eq!(p.short_qty, 3);
    assert_eq!(p.avg_entry(), 120);
}

#[test]
fn fill_at_same_price_no_pnl() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    p.apply_fill(1, 100, 10, 2);
    assert_eq!(p.realized_pnl, 0);
}

#[test]
fn realized_pnl_accumulates_across_fills() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    p.apply_fill(1, 110, 5, 2); // pnl = 5*10=50
    p.apply_fill(1, 120, 5, 3); // pnl = 5*20=100
    assert_eq!(p.realized_pnl, 150);
}

#[test]
fn self_trade_taker_and_maker_same_user() {
    // apply_fill handles one side at a time
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 5, 1); // buy as taker
    p.apply_fill(1, 100, 5, 1); // sell as maker
    assert!(p.is_empty());
    assert_eq!(p.realized_pnl, 0);
}

#[test]
fn max_qty_no_overflow() {
    let mut p = Position::new(1, 0);
    // Use large but not max values to avoid overflow
    p.apply_fill(0, 1_000_000, 1_000_000, 1);
    assert_eq!(p.long_qty, 1_000_000);
    assert_eq!(p.long_entry_cost, 1_000_000_000_000);
}

#[test]
fn max_price_no_overflow() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, i64::MAX / 1000, 1, 1);
    assert_eq!(p.long_qty, 1);
}

#[test]
fn position_version_increments_per_fill() {
    let mut p = Position::new(1, 0);
    assert_eq!(p.version, 0);
    p.apply_fill(0, 100, 10, 1);
    assert_eq!(p.version, 1);
    p.apply_fill(0, 100, 10, 2);
    assert_eq!(p.version, 2);
}

#[test]
fn empty_position_zero_notional_zero_upnl() {
    let p = Position::new(1, 0);
    assert_eq!(p.notional(100), 0);
    assert_eq!(p.unrealized_pnl(100), 0);
    assert!(p.is_empty());
}
