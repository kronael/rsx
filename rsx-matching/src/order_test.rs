use crate::tile_test_utils::decode_all;
use crate::tile_test_utils::order_message;
use crate::tile_test_utils::record_seqs;
use crate::tile_test_utils::record_types;
use crate::tile_test_utils::Tile;
use crate::tile_test_utils::BUY;
use crate::tile_test_utils::GATEWAY_IN_NS;
use crate::tile_test_utils::RISK_IN_NS;
use crate::tile_test_utils::SELL;
use rsx_book::event::OrderFailureReason;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::time_utils::time_ns;
use rsx_types::Price;
use rsx_types::Qty;

/// specs/2/17-matching.md:40-42, specs/2/47-validation-edge-cases.md:383:
/// the same order id twice -> one OrderFailed(Duplicate) on WAL, risk and
/// marketdata, no ACCEPTED, the first order untouched.
#[test]
fn duplicate_order_id_yields_only_order_failed_on_every_stream() {
    let mut tile = Tile::new();
    tile.submit(&order_message(7, 1, BUY, 49_999, 10));
    tile.take();

    tile.submit(&order_message(7, 1, BUY, 49_999, 10));
    let out = tile.take();

    let frames = out.streams_agree();
    assert_eq!(record_types(frames), [RECORD_ORDER_FAILED]);
    let failed = decode_all::<OrderFailedRecord>(frames, RECORD_ORDER_FAILED)[0];
    assert_eq!(
        (failed.user_id, failed.order_id_hi, failed.order_id_lo),
        (7, 0, 1)
    );
    assert_eq!(failed.reason, OrderFailureReason::Duplicate as u8);
    assert_eq!(tile.order_index.len(), 1);
    assert_eq!(tile.book.current_bbo().bid_qty, Qty(10));
}

/// specs/2/47-validation-edge-cases.md:384: a retry under a new order id
/// after a duplicate reject -> accepted as a new order.
#[test]
fn retry_with_a_new_order_id_is_accepted() {
    let mut tile = Tile::new();
    tile.submit(&order_message(7, 1, BUY, 49_999, 10));
    tile.submit(&order_message(7, 1, BUY, 49_999, 10));
    tile.take();

    tile.submit(&order_message(7, 2, BUY, 49_999, 10));
    let out = tile.take();

    let frames = out.streams_agree();
    assert_eq!(record_types(frames)[0], RECORD_ORDER_ACCEPTED);
    assert_eq!(tile.order_index.len(), 2);
}

/// specs/2/17-matching.md:43-45, 54-63, 107-120: a crossing order -> its
/// ACCEPTED record first, then the fill before both ORDER_DONEs, the BBO
/// last, one contiguous seq run, identical on WAL, risk and marketdata.
#[test]
fn crossing_order_publishes_accepted_then_fill_then_dones_then_bbo() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, SELL, 50_000, 10));
    let resting = tile.take();
    assert_eq!(
        record_types(resting.streams_agree()),
        [RECORD_ORDER_ACCEPTED, RECORD_ORDER_INSERTED, RECORD_BBO]
    );

    tile.submit(&order_message(2, 2, BUY, 50_000, 10));
    let out = tile.take();

    let frames = out.streams_agree();
    let types = record_types(frames);
    let fill_at = types.iter().position(|kind| *kind == RECORD_FILL).unwrap();
    assert_eq!(types[0], RECORD_ORDER_ACCEPTED);
    assert_eq!(types.iter().filter(|kind| **kind == RECORD_FILL).count(), 1);
    assert_eq!(
        types
            .iter()
            .filter(|kind| **kind == RECORD_ORDER_DONE)
            .count(),
        2
    );
    for (at, kind) in types.iter().enumerate() {
        if *kind == RECORD_ORDER_DONE {
            assert!(
                fill_at < at,
                "ORDER_DONE at {at} before the fill at {fill_at}"
            );
        }
    }
    assert_eq!(types.last(), Some(&RECORD_BBO));
    assert_eq!(record_seqs(frames), [4, 5, 6, 7, 8]);

    let accepted = decode_all::<OrderAcceptedRecord>(frames, RECORD_ORDER_ACCEPTED)[0];
    assert_eq!(
        (accepted.user_id, accepted.order_id_lo, accepted.side),
        (2, 2, BUY)
    );
    assert_eq!((accepted.price, accepted.qty), (50_000, 10));
    let fill = decode_all::<FillRecord>(frames, RECORD_FILL)[0];
    assert_eq!((fill.maker_user_id, fill.taker_user_id), (1, 2));
    assert_eq!((fill.price, fill.qty), (Price(50_000), Qty(10)));
    let bbo = decode_all::<BboRecord>(frames, RECORD_BBO)[0];
    assert_eq!((bbo.bid_qty, bbo.ask_qty), (Qty(0), Qty(0)));
}

/// specs/2/59-latency-observability.md:36-37, 40, 66-67: a fill and the
/// taker's ORDER_DONE carry the gateway and risk ingress stamps through and
/// `me_in_ns <= match_done_ns`, both inside the call; the maker's
/// ORDER_DONE has no match cycle of its own and stays zero.
#[test]
fn fill_and_taker_done_carry_hop_stamps_in_order() {
    let mut tile = Tile::new();
    tile.submit(&order_message(1, 1, SELL, 50_000, 10));
    tile.take();

    let before_ns = time_ns();
    tile.submit(&order_message(2, 2, BUY, 50_000, 10));
    let after_ns = time_ns();
    let out = tile.take();

    let frames = out.streams_agree();
    let fill = decode_all::<FillRecord>(frames, RECORD_FILL)[0];
    assert_eq!(fill.gw_in_ns, GATEWAY_IN_NS);
    assert_eq!(fill.risk_in_ns, RISK_IN_NS);
    assert!(before_ns <= fill.me_in_ns);
    assert!(fill.me_in_ns <= fill.match_done_ns);
    assert!(fill.match_done_ns <= after_ns);

    let dones = decode_all::<OrderDoneRecord>(frames, RECORD_ORDER_DONE);
    let taker = dones.iter().find(|done| done.user_id == 2).unwrap();
    assert_eq!(
        (taker.gw_in_ns, taker.risk_in_ns),
        (GATEWAY_IN_NS, RISK_IN_NS)
    );
    assert_eq!(
        (taker.me_in_ns, taker.match_done_ns),
        (fill.me_in_ns, fill.match_done_ns)
    );
    let maker = dones.iter().find(|done| done.user_id == 1).unwrap();
    assert_eq!(
        (maker.me_in_ns, maker.match_done_ns, maker.risk_in_ns),
        (0, 0, 0)
    );
}

/// specs/2/47-validation-edge-cases.md:460-461: one user's sell against the
/// same user's resting bid -> the fill executes and both orders complete.
#[test]
fn self_trade_executes_and_completes_both_orders() {
    let mut tile = Tile::new();
    tile.submit(&order_message(42, 1, BUY, 50_000, 10));
    tile.take();

    tile.submit(&order_message(42, 2, SELL, 50_000, 10));
    let out = tile.take();

    let frames = out.streams_agree();
    let fill = decode_all::<FillRecord>(frames, RECORD_FILL)[0];
    assert_eq!((fill.maker_user_id, fill.taker_user_id), (42, 42));
    assert_eq!((fill.maker_order_id_lo, fill.taker_order_id_lo), (1, 2));
    assert_eq!((fill.price, fill.qty), (Price(50_000), Qty(10)));
    let dones = decode_all::<OrderDoneRecord>(frames, RECORD_ORDER_DONE);
    let mut done_ids: Vec<u64> = dones.iter().map(|done| done.order_id_lo).collect();
    done_ids.sort();
    assert_eq!(done_ids, [1, 2]);
    assert!(tile.order_index.is_empty());
}
