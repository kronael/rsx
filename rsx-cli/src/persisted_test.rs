use crate::check::Status;
use crate::fixture_utils::Wal;
use crate::fixture_utils::BUY;
use crate::fixture_utils::SELL;
use crate::fixture_utils::SYMBOL;
use crate::ledger::Ledger;
use crate::persisted::check_positions;
use crate::persisted::check_tips;
use crate::persisted::tally_funding;
use crate::persisted::tally_locks;
use crate::persisted::IntervalSum;
use crate::persisted::Persisted;
use rsx_risk::Position;
use rustc_hash::FxHashMap;
use std::path::Path;

const FILL_SEQ: u64 = 4;

fn filled_cycle() -> Wal {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, SELL, 100, 5)
        .fill((2, 2), (1, 1), 100, 5, SELL)
        .done(1, 1)
        .done(2, 2);
    wal
}

fn ledger_with<'a>(wal: &Wal, persisted: &'a Persisted) -> Ledger<'a> {
    let mut ledger = Ledger::new(Path::new("wal"), SYMBOL, Some(persisted));
    for (record_type, payload) in &wal.records {
        ledger.apply(*record_type, payload);
    }
    ledger.finish();
    ledger
}

fn position(user_id: u32, side: u8, seq: u64) -> Position {
    let mut position = Position::new(user_id, SYMBOL);
    position.apply_fill(side, 100, 5, seq);
    position
}

fn rows(positions: Vec<Position>) -> FxHashMap<(u32, u32), Position> {
    positions
        .into_iter()
        .map(|position| ((position.user_id, position.symbol_id), position))
        .collect()
}

fn persisted_after_the_fill() -> Persisted {
    Persisted::new(
        rows(vec![
            position(1, BUY, FILL_SEQ),
            position(2, SELL, FILL_SEQ),
        ]),
        vec![(0, SYMBOL, 6)],
    )
}

#[test]
fn positions_that_equal_the_sum_of_fills_pass() {
    let persisted = persisted_after_the_fill();
    let ledger = ledger_with(&filled_cycle(), &persisted);
    let check = check_positions(&persisted, &[ledger]);
    assert_eq!(check.status, Status::Pass);
    assert_eq!(check.checked, 4);
}

#[test]
fn a_persisted_position_that_differs_from_the_fills_fails() {
    let mut wrong = position(1, BUY, FILL_SEQ);
    wrong.long_qty = 4;
    let persisted = Persisted::new(
        rows(vec![wrong, position(2, SELL, FILL_SEQ)]),
        vec![(0, SYMBOL, 6)],
    );
    let ledger = ledger_with(&filled_cycle(), &persisted);
    let check = check_positions(&persisted, &[ledger]);
    assert_eq!(check.status, Status::Fail);
    assert_eq!(check.violation_count, 1);
    let violation = &check.violations[0];
    assert_eq!(violation["user_id"], 1);
    assert_eq!(violation["wal"]["long_qty"], 5);
    assert_eq!(violation["postgres"]["long_qty"], 4);
}

#[test]
fn a_fill_risk_tipped_past_with_no_position_row_fails() {
    let persisted = Persisted::new(rows(vec![position(1, BUY, FILL_SEQ)]), vec![(0, SYMBOL, 6)]);
    let ledger = ledger_with(&filled_cycle(), &persisted);
    let check = check_positions(&persisted, &[ledger]);
    assert_eq!(check.status, Status::Fail);
    assert_eq!(check.violations[0]["user_id"], 2);
    assert_eq!(check.violations[0]["fill_seq"], FILL_SEQ);
}

#[test]
fn a_fill_beyond_the_lowest_tip_may_still_be_missing_from_postgres() {
    let persisted = Persisted::new(rows(vec![]), vec![(0, SYMBOL, 3)]);
    let ledger = ledger_with(&filled_cycle(), &persisted);
    let check = check_positions(&persisted, &[ledger]);
    assert_eq!(check.status, Status::Skip);
}

#[test]
fn a_position_is_summed_only_up_to_its_last_fill_seq() {
    let mut wal = filled_cycle();
    wal.accepted(1, 3, BUY, 100, 5)
        .inserted(1, 3, BUY, 100, 5)
        .accepted(2, 4, SELL, 100, 5)
        .fill((2, 4), (1, 3), 100, 5, SELL)
        .done(1, 3)
        .done(2, 4);
    let persisted = Persisted::new(
        rows(vec![
            position(1, BUY, FILL_SEQ),
            position(2, SELL, FILL_SEQ),
        ]),
        vec![(0, SYMBOL, FILL_SEQ)],
    );
    let ledger = ledger_with(&wal, &persisted);
    let check = check_positions(&persisted, &[ledger]);
    assert_eq!(check.status, Status::Pass);
}

#[test]
fn a_stream_with_a_pruned_head_cannot_be_summed() {
    let mut wal = Wal::starting_at(7);
    wal.accepted(2, 2, SELL, 100, 5)
        .fill((2, 2), (1, 1), 100, 5, SELL)
        .done(2, 2);
    let persisted = persisted_after_the_fill();
    let ledger = ledger_with(&wal, &persisted);
    let check = check_positions(&persisted, &[ledger]);
    assert_eq!(check.status, Status::Skip);
    assert!(check.detail.contains("no WAL stream from seq 1"));
}

#[test]
fn a_tip_past_the_last_wal_seq_fails() {
    let persisted = Persisted::new(rows(vec![]), vec![(0, SYMBOL, 6), (1, SYMBOL, 7)]);
    let check = check_tips(&persisted, &[(SYMBOL, 6)]);
    assert_eq!(check.status, Status::Fail);
    assert_eq!(check.checked, 2);
    assert_eq!(check.violations[0]["instance_id"], 1);
}

#[test]
fn tips_within_the_wal_pass_and_a_symbol_without_a_stream_is_not_examined() {
    let persisted = Persisted::new(rows(vec![]), vec![(0, SYMBOL, 6), (0, 9, 99)]);
    let check = check_tips(&persisted, &[(SYMBOL, 6)]);
    assert_eq!(check.status, Status::Pass);
    assert_eq!(check.checked, 1);
}

fn sum(shards_settled: u64, total: i128) -> IntervalSum {
    IntervalSum {
        symbol_id: 1,
        interval_id: 5,
        shards_settled,
        total,
    }
}

#[test]
fn one_shard_must_sum_to_exactly_zero() {
    assert_eq!(tally_funding(&[sum(1, 0)], 1).0.violation_count, 0);
    assert_eq!(tally_funding(&[sum(1, 10)], 1).0.violation_count, 1);
    assert_eq!(tally_funding(&[sum(1, -1)], 1).0.violation_count, 1);
}

#[test]
fn shards_may_round_down_by_one_unit_each_but_never_pay_out_net() {
    assert_eq!(tally_funding(&[sum(2, -1)], 2).0.violation_count, 0);
    assert_eq!(tally_funding(&[sum(2, -2)], 2).0.violation_count, 1);
    assert_eq!(tally_funding(&[sum(2, 1)], 2).0.violation_count, 1);
}

#[test]
fn an_interval_some_shard_has_not_settled_is_not_examined() {
    let (tally, unsettled) = tally_funding(&[sum(1, 999)], 2);
    assert_eq!(tally.checked, 0);
    assert_eq!(tally.violation_count, 0);
    assert_eq!(unsettled, 1);
}

#[test]
fn a_lock_key_with_two_holders_is_flagged() {
    let tally = tally_locks(&[(0, 1), (7, 2)]);
    assert_eq!(tally.checked, 2);
    assert_eq!(tally.violation_count, 1);
    assert_eq!(tally.violations[0]["lock_key"], 7);
}
