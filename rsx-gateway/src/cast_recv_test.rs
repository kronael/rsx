use crate::cast_recv::decode_cast_event;
use crate::cast_recv::CastEvent;
use rsx_cast::as_bytes;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_LIQUIDATION;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::Price;
use rsx_types::Qty;

fn fill_record() -> FillRecord {
    FillRecord {
        seq: 1,
        ts_ns: 100,
        symbol_id: 10,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 11,
        taker_order_id_lo: 22,
        maker_order_id_hi: 33,
        maker_order_id_lo: 44,
        price: Price(30_000),
        qty: Qty(1_000),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

#[test]
fn decode_fill_dispatches_to_fill_variant() {
    let rec = fill_record();
    match decode_cast_event(RECORD_FILL, as_bytes(&rec)) {
        Some(CastEvent::Fill(decoded)) => {
            assert_eq!(decoded.taker_order_id_hi, rec.taker_order_id_hi);
            assert_eq!(decoded.price.0, rec.price.0);
        }
        _ => panic!("expected CastEvent::Fill"),
    }
}

#[test]
fn decode_order_inserted_dispatches_correctly() {
    let rec = OrderInsertedRecord {
        seq: 1,
        ts_ns: 100,
        symbol_id: 10,
        user_id: 5,
        order_id_hi: 1,
        order_id_lo: 2,
        price: Price(30_000),
        qty: Qty(1_000),
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    match decode_cast_event(RECORD_ORDER_INSERTED, as_bytes(&rec)) {
        Some(CastEvent::OrderInserted(decoded)) => {
            assert_eq!(decoded.user_id, rec.user_id);
        }
        _ => panic!("expected CastEvent::OrderInserted"),
    }
}

#[test]
fn decode_order_done_dispatches_correctly() {
    let rec = OrderDoneRecord {
        seq: 1,
        ts_ns: 100,
        symbol_id: 10,
        user_id: 5,
        order_id_hi: 1,
        order_id_lo: 2,
        filled_qty: Qty(1_000),
        remaining_qty: Qty(0),
        final_status: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    };
    match decode_cast_event(RECORD_ORDER_DONE, as_bytes(&rec)) {
        Some(CastEvent::OrderDone(decoded)) => {
            assert_eq!(decoded.order_id_hi, rec.order_id_hi);
        }
        _ => panic!("expected CastEvent::OrderDone"),
    }
}

#[test]
fn decode_order_cancelled_dispatches_correctly() {
    let rec = OrderCancelledRecord {
        seq: 1,
        ts_ns: 100,
        symbol_id: 10,
        user_id: 5,
        order_id_hi: 1,
        order_id_lo: 2,
        remaining_qty: Qty(500),
        reason: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    match decode_cast_event(RECORD_ORDER_CANCELLED, as_bytes(&rec)) {
        Some(CastEvent::OrderCancelled(decoded)) => {
            assert_eq!(decoded.order_id_lo, rec.order_id_lo);
        }
        _ => panic!("expected CastEvent::OrderCancelled"),
    }
}

#[test]
fn decode_order_failed_dispatches_correctly() {
    let rec = OrderFailedRecord {
        seq: 1,
        ts_ns: 100,
        user_id: 5,
        _pad0: 0,
        order_id_hi: 1,
        order_id_lo: 2,
        reason: 3,
        _pad: [0; 23],
    };
    match decode_cast_event(RECORD_ORDER_FAILED, as_bytes(&rec)) {
        Some(CastEvent::OrderFailed(decoded)) => {
            assert_eq!(decoded.reason, rec.reason);
        }
        _ => panic!("expected CastEvent::OrderFailed"),
    }
}

#[test]
fn decode_liquidation_dispatches_correctly() {
    let rec = LiquidationRecord {
        seq: 1,
        ts_ns: 100,
        user_id: 5,
        symbol_id: 10,
        status: 1,
        side: 0,
        _pad0: [0; 2],
        round: 1,
        qty: 1_000,
        price: 30_000,
        slip_bps: 5,
    };
    match decode_cast_event(RECORD_LIQUIDATION, as_bytes(&rec)) {
        Some(CastEvent::Liquidation(decoded)) => {
            assert_eq!(decoded.round, rec.round);
        }
        _ => panic!("expected CastEvent::Liquidation"),
    }
}

#[test]
fn decode_config_applied_extracts_symbol_and_version() {
    let rec = ConfigAppliedRecord {
        seq: 1,
        ts_ns: 100,
        symbol_id: 10,
        _pad0: 0,
        config_version: 7,
        effective_at_ms: 0,
        applied_at_ns: 0,
    };
    match decode_cast_event(RECORD_CONFIG_APPLIED, as_bytes(&rec)) {
        Some(CastEvent::ConfigApplied {
            symbol_id,
            config_version,
        }) => {
            assert_eq!(symbol_id, 10);
            assert_eq!(config_version, 7);
        }
        _ => panic!("expected CastEvent::ConfigApplied"),
    }
}

#[test]
fn decode_unknown_record_type_returns_none() {
    let payload = [0u8; 128];
    assert!(decode_cast_event(0xFFFF, &payload).is_none());
}

#[test]
fn decode_truncated_payload_returns_none() {
    let rec = fill_record();
    let bytes = as_bytes(&rec);
    // Shorter than size_of::<FillRecord>() -- decode_payload must abstain.
    assert!(decode_cast_event(RECORD_FILL, &bytes[..8]).is_none());
}
