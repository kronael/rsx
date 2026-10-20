use crate::tile_test_utils::decode_all;
use crate::tile_test_utils::dirty_stack;
use crate::tile_test_utils::order_message;
use crate::tile_test_utils::record_types;
use crate::tile_test_utils::Tile;
use crate::tile_test_utils::BUY;
use crate::tile_test_utils::SELL;
use rsx_book::event::OrderFailureReason;
use rsx_messages::BboRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::CANCEL_REASON_USER_CANCEL;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_types::Price;
use rsx_types::Qty;

const CANCELLED_STATUS: u8 = 2;

/// specs/2/17-matching.md:67-69, 99-105 and specs/2/47-validation-edge-cases.md:411:
/// a cancel of the best resting bid -> OrderCancelled, OrderDone(cancelled)
/// and the BBO that follows from the new best, with no ACCEPTED record,
/// identical on WAL, risk and marketdata.
#[test]
fn cancel_of_best_bid_publishes_cancelled_done_and_new_bbo() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, BUY, 50_000, 10));
    tile.submit(&order_message(1, 2, BUY, 49_999, 5));
    tile.take();

    tile.cancel(1, 1);
    let out = tile.take();

    let frames = out.streams_agree();
    assert_eq!(
        record_types(frames),
        [RECORD_ORDER_CANCELLED, RECORD_ORDER_DONE, RECORD_BBO]
    );
    let cancelled = decode_all::<OrderCancelledRecord>(frames, RECORD_ORDER_CANCELLED)[0];
    assert_eq!(
        (
            cancelled.user_id,
            cancelled.order_id_hi,
            cancelled.order_id_lo
        ),
        (1, 0, 1)
    );
    assert_eq!(cancelled.remaining_qty, Qty(10));
    assert_eq!(cancelled.reason, CANCEL_REASON_USER_CANCEL);
    let done = decode_all::<OrderDoneRecord>(frames, RECORD_ORDER_DONE)[0];
    assert_eq!((done.user_id, done.order_id_lo), (1, 1));
    assert_eq!(done.final_status, CANCELLED_STATUS);
    assert_eq!((done.filled_qty, done.remaining_qty), (Qty(0), Qty(10)));
    let bbo = decode_all::<BboRecord>(frames, RECORD_BBO)[0];
    assert_eq!((bbo.bid_px, bbo.bid_qty), (Price(49_999), Qty(5)));
    assert_eq!((bbo.ask_px, bbo.ask_qty), (Price(0), Qty(0)));
    assert!(!tile.order_index.contains_key(&(1, 0, 1)));
    assert!(tile.order_index.contains_key(&(1, 0, 2)));
}

/// specs/2/59-latency-observability.md:45-46: a cancel has no match cycle,
/// so its OrderDone carries no hop stamps.
#[test]
fn cancel_done_carries_no_hop_stamps() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, BUY, 50_000, 10));
    tile.take();

    tile.cancel(1, 1);
    let out = tile.take();

    let done = decode_all::<OrderDoneRecord>(out.streams_agree(), RECORD_ORDER_DONE)[0];
    assert_eq!(
        (
            done.gw_in_ns,
            done.risk_in_ns,
            done.me_in_ns,
            done.match_done_ns
        ),
        (0, 0, 0, 0)
    );
}

/// specs/2/17-matching.md:99-105: cancelling the last resting order empties
/// the book -> a BBO with both sides zero.
#[test]
fn cancel_of_the_last_order_publishes_an_empty_bbo() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, SELL, 50_001, 10));
    tile.take();

    tile.cancel(1, 1);
    let out = tile.take();

    let bbo = decode_all::<BboRecord>(out.streams_agree(), RECORD_BBO)[0];
    assert_eq!((bbo.bid_px, bbo.bid_qty), (Price(0), Qty(0)));
    assert_eq!((bbo.ask_px, bbo.ask_qty), (Price(0), Qty(0)));
}

/// specs/2/17-matching.md:71-79: cancelling a partly filled order -> the
/// cancel reports the quantity still resting.
#[test]
fn cancel_of_a_partly_filled_order_reports_remaining_qty() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, SELL, 50_000, 10));
    tile.submit(&order_message(2, 2, BUY, 50_000, 4));
    tile.take();

    tile.cancel(1, 1);
    let out = tile.take();

    let frames = out.streams_agree();
    let cancelled = decode_all::<OrderCancelledRecord>(frames, RECORD_ORDER_CANCELLED)[0];
    assert_eq!(cancelled.remaining_qty, Qty(6));
    let done = decode_all::<OrderDoneRecord>(frames, RECORD_ORDER_DONE)[0];
    assert_eq!(done.remaining_qty, Qty(6));
}

/// specs/2/47-validation-edge-cases.md:412-415 (filled, never existed,
/// wrong user, already cancelled): a cancel that finds no order of that
/// user changes nothing, cannot touch another user's order and, until
/// BUGS.md ME-CANCEL-MISS-SILENT is decided, publishes nothing.
#[test]
fn cancel_misses_leave_the_book_and_every_stream_untouched() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, BUY, 50_000, 10));
    tile.submit(&order_message(2, 2, SELL, 50_000, 10));
    tile.submit(&order_message(4, 4, BUY, 49_000, 5));
    tile.take();

    tile.cancel(1, 1);
    tile.cancel(9, 99);
    tile.cancel(5, 4);
    let out = tile.take();

    assert!(out.wal.is_empty());
    assert!(out.risk.is_empty());
    assert!(out.mkt.is_empty());
    assert_eq!(tile.order_index.len(), 1);
    assert_eq!(tile.book.current_bbo().bid_qty, Qty(5));

    tile.cancel(4, 4);
    tile.take();
    tile.cancel(4, 4);
    let again = tile.take();
    assert!(again.wal.is_empty());
    assert!(again.risk.is_empty());
    assert!(again.mkt.is_empty());
}

/// specs/2/47-validation-edge-cases.md:385: CANCEL(X) then NEW(X) inside the
/// dedup window -> OrderFailed(Duplicate), nothing rests.
#[test]
fn order_id_is_not_reusable_after_cancel() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, BUY, 50_000, 10));
    tile.cancel(1, 1);
    tile.take();

    tile.submit(&order_message(1, 1, BUY, 50_000, 10));
    let out = tile.take();

    let frames = out.streams_agree();
    assert_eq!(record_types(frames), [RECORD_ORDER_FAILED]);
    let failed = decode_all::<OrderFailedRecord>(frames, RECORD_ORDER_FAILED)[0];
    assert_eq!(failed.reason, OrderFailureReason::Duplicate as u8);
    assert!(tile.order_index.is_empty());
}

/// Every payload byte is deterministic: the 8 bytes after `_pad1` of an
/// OrderCancelledRecord are tail padding the ME never writes, and they must
/// be zero rather than whatever the stack held. Fails today (garbage reaches
/// the WAL and both streams).
#[test]
#[ignore = "BUGS.md ME-WIRE-RECORD-TAIL-PADDING-UNINITIALISED: tail padding is stack garbage"]
fn cancelled_record_tail_padding_is_zero() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, BUY, 50_000, 10));
    tile.take();

    dirty_stack();
    tile.cancel(1, 1);
    let out = tile.take();

    let (_, payload) = &out.streams_agree()[0];
    let padding_at = std::mem::offset_of!(OrderCancelledRecord, _pad1) + 4;
    assert_eq!(payload[padding_at..], [0u8; 8]);
}
