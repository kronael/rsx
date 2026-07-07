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
    p.apply_fill(1, 110, 5, 2); // sell 5
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
    assert_eq!(p.notional(100).unwrap(), 0);
    assert_eq!(p.unrealized_pnl(100).unwrap(), 0);
    assert!(p.is_empty());
}

// -- unrealized PnL sign per side (RISK.md §3) --

#[test]
fn unrealized_pnl_long_sign() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1); // long 10 @ 100
                                 // long uPnL = net_qty * (mark - avg) = 10 * (130-100)
    assert_eq!(p.unrealized_pnl(130).unwrap(), 300);
    assert_eq!(p.unrealized_pnl(80).unwrap(), -200);
}

#[test]
fn unrealized_pnl_short_sign() {
    let mut p = Position::new(1, 0);
    p.apply_fill(1, 100, 10, 1); // short 10 @ 100
                                 // short uPnL = net_qty * (mark - avg) = -10 * (mark-100)
                                 // mark below entry -> profit; above -> loss
    assert_eq!(p.unrealized_pnl(80).unwrap(), 200);
    assert_eq!(p.unrealized_pnl(130).unwrap(), -300);
}

#[test]
fn notional_uses_abs_net_qty_both_sides() {
    let mut p = Position::new(1, 0);
    p.apply_fill(1, 100, 7, 1); // short 7
                                // notional = |net_qty| * mark = 7 * 50
    assert_eq!(p.notional(50).unwrap(), 350);
}

// -- flip from a non-round accumulated average --

#[test]
fn flip_from_accumulated_nonround_avg() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 3, 1); // long 3 @ 100, cost 300
    p.apply_fill(0, 107, 4, 2); // +4 @ 107, cost 428
                                // long_qty=7, cost=728, avg = 728/7 = 104 (truncated)
    assert_eq!(p.long_qty, 7);
    assert_eq!(p.long_entry_cost, 728);
    assert_eq!(p.avg_entry(), 104);
    p.apply_fill(1, 120, 10, 3); // sell 10: close 7 long, open 3 short
                                 // realized = 120*7 - 728 = 112
    assert_eq!(p.realized_pnl, 112);
    assert_eq!(p.long_qty, 0);
    assert_eq!(p.long_entry_cost, 0); // closed side fully zeroed
    assert_eq!(p.short_qty, 3);
    assert_eq!(p.short_entry_cost, 360); // 3 * 120
    assert_eq!(p.net_qty(), -3);
    assert_eq!(p.avg_entry(), 120); // new short opened at fill price
}

// -- partial close of a short with truncating cost split --

#[test]
fn partial_close_short_truncates_cost_keeps_avg() {
    let mut p = Position::new(1, 0);
    p.apply_fill(1, 100, 3, 1); // short 3, proceeds 300
    p.apply_fill(1, 107, 4, 2); // short +4, proceeds 428
                                // short_qty=7, cost=728, avg=104
    assert_eq!(p.short_qty, 7);
    assert_eq!(p.short_entry_cost, 728);
    p.apply_fill(0, 90, 3, 3); // buy 3 to close part of short, at a profit
                               // close_cost = 728*3/7 = 312 (truncated); realized = 312 - 90*3 = 42
    assert_eq!(p.realized_pnl, 42);
    assert_eq!(p.short_qty, 4);
    assert_eq!(p.short_entry_cost, 416); // 728 - 312
    assert_eq!(p.avg_entry(), 104); // 416/4 = 104, basis preserved
}

// -- repeated partial closes zero out cost exactly at zero qty --

#[test]
fn repeated_partial_closes_zero_cost_at_zero_qty() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 101, 7, 1); // long 7 @ 101, cost 707 (avg 101)
    p.apply_fill(1, 110, 3, 2); // close 3
    p.apply_fill(1, 95, 1, 3); // close 1
    p.apply_fill(1, 120, 3, 4); // close remaining 3 -> exactly flat
    assert!(p.is_empty());
    assert_eq!(p.net_qty(), 0);
    // Closed side cost MUST be exactly zero (no truncation residue at flat).
    assert_eq!(p.long_entry_cost, 0);
    assert_eq!(p.short_entry_cost, 0);
    assert_eq!(p.avg_entry(), 0);
    assert_eq!(p.unrealized_pnl(999).unwrap(), 0);
}

// -- flat position with realized PnL still reports zero uPnL/notional --

#[test]
fn flat_with_realized_pnl_zero_upnl() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    p.apply_fill(1, 130, 10, 2); // close for +300 realized
    assert_eq!(p.realized_pnl, 300);
    assert!(p.is_empty());
    assert_eq!(p.unrealized_pnl(500).unwrap(), 0);
    assert_eq!(p.notional(500).unwrap(), 0);
    assert_eq!(p.avg_entry(), 0);
}

// -- net_qty = long_qty - short_qty sign --

#[test]
fn net_qty_sign_long_positive_short_negative() {
    let mut long = Position::new(1, 0);
    long.apply_fill(0, 100, 5, 1);
    assert_eq!(long.net_qty(), 5);
    let mut short = Position::new(1, 0);
    short.apply_fill(1, 100, 5, 1);
    assert_eq!(short.net_qty(), -5);
}
