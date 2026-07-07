use crate::order_id::order_id_to_hex;
use crate::records::serialize;
use crate::records::WsFrame;
use crate::state::GatewayState;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn oid_bytes(hi: u64, lo: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes
}

fn oid_hex(hi: u64, lo: u64) -> String {
    order_id_to_hex(&oid_bytes(hi, lo))
}

/// Serialize `frame` once (into a shared `Arc<str>`) and queue it for every
/// connection of `user_id`. The single-recipient route_* handlers all share
/// this serialize -> push shape.
fn emit_to_user(state: &Rc<RefCell<GatewayState>>, user_id: u32, frame: &WsFrame) {
    let msg: Arc<str> = serialize(frame).into();
    state.borrow_mut().push_to_user(user_id, msg);
}

pub fn route_fill(state: &Rc<RefCell<GatewayState>>, rec: &FillRecord) {
    let taker_oid = oid_hex(rec.taker_order_id_hi, rec.taker_order_id_lo);
    let maker_oid = oid_hex(rec.maker_order_id_hi, rec.maker_order_id_lo);
    let msg = serialize(&WsFrame::Fill {
        taker_order_id: taker_oid,
        maker_order_id: maker_oid,
        price: rec.price.0,
        qty: rec.qty.0,
        timestamp_ns: rec.ts_ns,
        fee: 0, // v1: fee not in FillRecord, computed at risk layer
    });
    // F4.3 — per-stage latency trace. Stage `gateway_out`
    // closes the GW→ME→GW loop. Anchor against the taker's
    // gateway-ingress timestamp (rec.taker_ts_ns) so the
    // closing delta composes with gateway_in / risk_in /
    // me_in / me_out / risk_out on the same clock origin.
    // Falls back to rec.ts_ns (ME emit) for legacy records.
    // serialize_done is captured before gateway_out to
    // attribute the serde_json cost separately.
    rsx_log::latency_sample!(
        "gateway_route_serialize_done",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        if rec.taker_ts_ns > 1_700_000_000_000_000_000 {
            rec.taker_ts_ns
        } else {
            rec.ts_ns
        }
    );
    rsx_log::latency_sample!(
        "gateway_out",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        if rec.taker_ts_ns > 1_700_000_000_000_000_000 {
            rec.taker_ts_ns
        } else {
            rec.ts_ns
        }
    );
    let msg: Arc<str> = msg.into();
    let mut st = state.borrow_mut();
    st.push_to_user(rec.taker_user_id, msg.clone());
    st.push_to_user(rec.maker_user_id, msg);
    drop(st);
    rsx_log::latency_sample!(
        "gateway_route_push_done",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        if rec.taker_ts_ns > 1_700_000_000_000_000_000 {
            rec.taker_ts_ns
        } else {
            rec.ts_ns
        }
    );
}

pub fn route_order_inserted(state: &Rc<RefCell<GatewayState>>, rec: &OrderInsertedRecord) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    emit_to_user(
        state,
        rec.user_id,
        &WsFrame::OrderUpdate {
            order_id: oid,
            status: 1, // resting/accepted from matching
            filled_qty: 0,
            remaining_qty: rec.qty.0,
            reason: 0,
        },
    );
}

pub fn route_order_done(state: &Rc<RefCell<GatewayState>>, rec: &OrderDoneRecord) {
    let status = match rec.final_status {
        0 => 0, // filled
        1 => 1, // resting (unexpected for done)
        2 => 2, // cancelled
        _ => 0,
    };
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    // Pair on order_id (see route_order_cancelled): the tracked
    // pending's user_id is authoritative, not rec.user_id.
    let removed = state
        .borrow_mut()
        .pending
        .remove(&oid_bytes(rec.order_id_hi, rec.order_id_lo));
    let user_id = removed.map_or(rec.user_id, |p| p.user_id);
    emit_to_user(
        state,
        user_id,
        &WsFrame::OrderUpdate {
            order_id: oid,
            status,
            filled_qty: rec.filled_qty.0,
            remaining_qty: rec.remaining_qty.0,
            reason: 0,
        },
    );
}

pub fn route_order_cancelled(state: &Rc<RefCell<GatewayState>>, rec: &OrderCancelledRecord) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    // Pair the completion on order_id, not the record's user_id:
    // a wrong user_id must not misroute the update or evict the
    // real owner's pending. Remove returns the tracked pending,
    // whose user_id is authoritative; fall back to rec.user_id
    // only when no pending exists (nothing to misroute/evict).
    let removed = state
        .borrow_mut()
        .pending
        .remove(&oid_bytes(rec.order_id_hi, rec.order_id_lo));
    let user_id = removed.map_or(rec.user_id, |p| p.user_id);
    emit_to_user(
        state,
        user_id,
        &WsFrame::OrderUpdate {
            order_id: oid,
            status: 2, // cancelled
            filled_qty: 0,
            remaining_qty: rec.remaining_qty.0,
            reason: 0,
        },
    );
}

pub fn route_liquidation(state: &Rc<RefCell<GatewayState>>, rec: &LiquidationRecord) {
    emit_to_user(
        state,
        rec.user_id,
        &WsFrame::Liquidation {
            symbol_id: rec.symbol_id,
            status: rec.status,
            round: rec.round,
            side: rec.side,
            qty: rec.qty,
            price: rec.price,
            slip_bps: rec.slip_bps,
        },
    );
}

pub fn route_order_failed(state: &Rc<RefCell<GatewayState>>, rec: &OrderFailedRecord) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    // Pair on order_id (see route_order_cancelled): the tracked
    // pending's user_id is authoritative, not rec.user_id.
    let removed = state
        .borrow_mut()
        .pending
        .remove(&oid_bytes(rec.order_id_hi, rec.order_id_lo));
    let user_id = removed.map_or(rec.user_id, |p| p.user_id);
    emit_to_user(
        state,
        user_id,
        &WsFrame::OrderUpdate {
            order_id: oid,
            status: 3, // failed
            filled_qty: 0,
            remaining_qty: 0,
            reason: rec.reason,
        },
    );
}
