use crate::fixture_utils::Wal;
use crate::fixture_utils::BUY;
use crate::fixture_utils::SELL;
use crate::fixture_utils::SYMBOL;
use crate::ledger::Ledger;
use rsx_cast::wal::extract_seq;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_types::FailureReason;
use std::path::Path;

fn ledger_of(wal: &Wal) -> Ledger<'static> {
    let mut ledger = Ledger::new(Path::new("wal"), SYMBOL, None);
    for (record_type, payload) in &wal.records {
        ledger.apply(*record_type, payload);
    }
    ledger.finish();
    ledger
}

fn playground_sorted_inversion_count(seqs: &[u64]) -> usize {
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).filter(|pair| pair[0] > pair[1]).count()
}

fn fill_seqs(wal: &Wal) -> Vec<u64> {
    wal.records
        .iter()
        .filter(|(record_type, _)| *record_type == RECORD_FILL)
        .filter_map(|(_, payload)| extract_seq(payload))
        .collect()
}

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

#[test]
fn a_clean_cycle_passes_every_wal_check() {
    let ledger = ledger_of(&filled_cycle());
    assert_eq!(ledger.intact.violation_count, 0);
    assert_eq!(ledger.monotonic.violation_count, 0);
    assert_eq!(ledger.fills_before_done.violation_count, 0);
    assert_eq!(ledger.completion.violation_count, 0);
    assert_eq!(ledger.fifo.violation_count, 0);
    assert!(ledger.fills_before_done.checked > 0);
    assert!(ledger.completion.checked > 0);
    assert!(ledger.fifo.checked > 0);
    assert!(ledger.is_cycle_complete());
}

#[test]
fn fill_after_order_done_is_flagged_though_the_sorted_fill_seqs_look_clean() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, SELL, 100, 5)
        .done(1, 1)
        .fill((2, 2), (1, 1), 100, 5, SELL)
        .done(2, 2);

    assert_eq!(playground_sorted_inversion_count(&fill_seqs(&wal)), 0);

    let ledger = ledger_of(&wal);
    assert_eq!(ledger.fills_before_done.violation_count, 1);
    let violation = &ledger.fills_before_done.violations[0];
    assert_eq!(violation["seq"], 5);
    assert_eq!(violation["order_done_seq"], 4);
    assert_eq!(violation["role"], "maker");
    assert_eq!(violation["user_id"], 1);
}

#[test]
fn a_second_order_done_is_a_second_completion() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, SELL, 100, 5).done(1, 1).done(1, 1);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.completion.violation_count, 1);
    assert_eq!(ledger.completion.violations[0]["seq"], 3);
}

#[test]
fn an_order_failed_after_an_order_done_is_a_second_completion() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, SELL, 100, 5)
        .done(1, 1)
        .failed(1, 1, FailureReason::InvalidLotSize as u8);
    assert_eq!(ledger_of(&wal).completion.violation_count, 1);
}

#[test]
fn an_accepted_order_with_no_insert_or_completion_is_stranded() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .accepted(1, 2, BUY, 99, 5)
        .inserted(1, 2, BUY, 99, 5);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.completion.violation_count, 1);
    assert_eq!(ledger.completion.violations[0]["seq"], 1);
}

#[test]
fn an_order_cut_off_by_the_end_of_the_stream_is_not_stranded() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.completion.violation_count, 0);
    assert!(!ledger.is_cycle_complete());
}

#[test]
fn a_cancel_without_its_order_done_is_stranded() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .cancelled(1, 1)
        .accepted(1, 2, BUY, 99, 5)
        .inserted(1, 2, BUY, 99, 5);
    assert_eq!(ledger_of(&wal).completion.violation_count, 1);
}

#[test]
fn a_cancel_of_a_resting_order_completes_through_its_order_done() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .cancelled(1, 1)
        .done(1, 1);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.completion.violation_count, 0);
    assert!(ledger.is_cycle_complete());
    assert_eq!(ledger.open_orders().count(), 0);
}

#[test]
fn a_post_only_cross_completes_through_its_order_done() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).done(1, 1);
    assert_eq!(ledger_of(&wal).completion.violation_count, 0);
}

#[test]
fn a_cancel_of_an_order_that_never_rested_is_a_violation() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).cancelled(1, 1);
    assert_eq!(ledger_of(&wal).completion.violation_count, 1);
}

#[test]
fn a_duplicate_reject_does_not_complete_the_original_order() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, SELL, 100, 5)
        .done(1, 1)
        .failed(1, 1, FailureReason::DuplicateOrderId as u8);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.completion.violation_count, 0);
    assert_eq!(ledger.duplicates, 1);
}

#[test]
fn a_record_about_an_unknown_order_fails_only_in_an_anchored_stream() {
    let mut anchored = Wal::default();
    anchored.done(9, 9);
    assert_eq!(ledger_of(&anchored).completion.violation_count, 1);

    let mut pruned = Wal::starting_at(7);
    pruned.done(9, 9);
    assert_eq!(ledger_of(&pruned).completion.violation_count, 0);
}

#[test]
fn an_order_accepted_again_before_it_completed_is_flagged() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5);
    assert_eq!(ledger_of(&wal).completion.violation_count, 1);
}

#[test]
fn fills_in_insertion_order_keep_time_priority_across_partial_fills() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, BUY, 100, 5)
        .inserted(2, 2, BUY, 100, 5)
        .accepted(3, 3, SELL, 100, 3)
        .fill((3, 3), (1, 1), 100, 3, SELL)
        .done(3, 3)
        .accepted(3, 4, SELL, 100, 7)
        .fill((3, 4), (1, 1), 100, 2, SELL)
        .done(1, 1)
        .fill((3, 4), (2, 2), 100, 5, SELL)
        .done(2, 2)
        .done(3, 4);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.fifo.violation_count, 0);
    assert_eq!(ledger.fifo.checked, 3);
    assert_eq!(ledger.completion.violation_count, 0);
}

#[test]
fn a_fill_that_skips_an_earlier_order_at_the_price_breaks_time_priority() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, BUY, 100, 5)
        .inserted(2, 2, BUY, 100, 5)
        .accepted(3, 3, SELL, 100, 5)
        .fill((3, 3), (2, 2), 100, 5, SELL)
        .done(2, 2)
        .done(3, 3);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.fifo.violation_count, 1);
    let violation = &ledger.fifo.violations[0];
    assert_eq!(violation["seq"], 6);
    assert_eq!(violation["user_id"], 2);
    assert_eq!(violation["earlier_user_id"], 1);
    assert_eq!(violation["earlier_inserted_seq"], 2);
}

#[test]
fn a_fill_against_a_maker_that_left_the_book_is_flagged() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .cancelled(1, 1)
        .done(1, 1)
        .accepted(2, 2, SELL, 100, 5)
        .fill((2, 2), (1, 1), 100, 5, SELL);
    assert_eq!(ledger_of(&wal).fifo.violation_count, 1);
}

#[test]
fn a_fill_at_another_price_than_the_resting_price_is_flagged() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, SELL, 99, 5)
        .fill((2, 2), (1, 1), 99, 5, SELL);
    assert_eq!(ledger_of(&wal).fifo.violation_count, 1);
}

#[test]
fn a_seq_that_does_not_increase_is_flagged() {
    let mut wal = Wal::default();
    wal.bbo(99, 1, 101, 1).set_next_seq(1).bbo(99, 1, 101, 1);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.monotonic.violation_count, 1);
    assert_eq!(ledger.monotonic.checked, 1);
}

#[test]
fn a_bbo_with_the_bid_at_or_above_the_ask_is_flagged() {
    let mut wal = Wal::default();
    wal.bbo(100, 5, 101, 5)
        .bbo(101, 5, 101, 5)
        .bbo(102, 5, 101, 5);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.bbo.checked, 3);
    assert_eq!(ledger.bbo.violation_count, 2);
}

#[test]
fn a_one_sided_bbo_is_not_examined() {
    let mut wal = Wal::default();
    wal.bbo(100, 5, 0, 0).bbo(0, 0, 101, 5);
    assert_eq!(ledger_of(&wal).bbo.checked, 0);
}

#[test]
fn a_payload_shorter_than_its_record_is_flagged() {
    let mut wal = Wal::default();
    wal.raw(RECORD_ORDER_DONE, 16);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.intact.violation_count, 1);
    assert_eq!(ledger.intact.checked, 1);
}

#[test]
fn a_read_error_is_reported_with_the_last_seq_read() {
    let mut ledger = Ledger::new(Path::new("wal"), SYMBOL, None);
    ledger.note_read_error("crc mismatch");
    assert_eq!(ledger.intact.violation_count, 1);
}

#[test]
fn the_peak_of_resting_orders_follows_inserts_and_exits() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(1, 2, BUY, 99, 5)
        .inserted(1, 2, BUY, 99, 5)
        .cancelled(1, 1)
        .done(1, 1)
        .accepted(1, 3, BUY, 98, 5)
        .inserted(1, 3, BUY, 98, 5);
    let ledger = ledger_of(&wal);
    assert_eq!(ledger.peak_resting, 2);
    assert_eq!(ledger.open_orders().count(), 2);
}
