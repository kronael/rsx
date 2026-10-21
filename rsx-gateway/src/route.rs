use crate::order_id::join_order_id;
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
use rsx_types::time_utils::time_ns;
use rsx_types::FailureReason;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tracing::warn;

fn oid_hex(hi: u64, lo: u64) -> String {
    order_id_to_hex(&join_order_id(hi, lo))
}

/// The wall-clock anchor of a fill's latency samples: the taker's gateway
/// ingress stamp, or the ME emit time for a record without one.
pub fn latency_anchor_ns(rec: &FillRecord) -> u64 {
    if rec.gw_in_ns > 1_700_000_000_000_000_000 {
        rec.gw_in_ns
    } else {
        rec.ts_ns
    }
}

/// An `OrderUpdate` frame with no latency stamps.
fn update(
    order_id: String,
    status: u8,
    filled_qty: i64,
    remaining_qty: i64,
    reason: u8,
) -> WsFrame {
    WsFrame::OrderUpdate {
        order_id,
        status,
        filled_qty,
        remaining_qty,
        reason,
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

/// Serialize `frame` once (into a shared `Arc<str>`) and queue it for every
/// connection of `user_id`.
fn emit_to_user(state: &Rc<RefCell<GatewayState>>, user_id: u32, frame: &WsFrame) {
    let msg: Arc<str> = serialize(frame).into();
    state.borrow_mut().push_to_user(user_id, msg);
}

/// Drop the order's pending entry and return the user to notify. The
/// tracked entry's user is authoritative: a record naming the wrong user
/// must not misroute the update or evict the owner's entry. Without an
/// entry the record's `user_id` stands.
fn complete_pending(state: &Rc<RefCell<GatewayState>>, user_id: u32, hi: u64, lo: u64) -> u32 {
    let removed = state
        .borrow_mut()
        .pending
        .remove(user_id, &join_order_id(hi, lo));
    removed.map_or(user_id, |pending| pending.user_id)
}

pub fn route_fill(state: &Rc<RefCell<GatewayState>>, rec: &FillRecord) {
    let msg: Arc<str> = serialize(&WsFrame::Fill {
        taker_order_id: oid_hex(rec.taker_order_id_hi, rec.taker_order_id_lo),
        maker_order_id: oid_hex(rec.maker_order_id_hi, rec.maker_order_id_lo),
        price: rec.price.0,
        qty: rec.qty.0,
        timestamp_ns: rec.ts_ns,
        fee: 0,
        gw_in_ns: rec.gw_in_ns,
        risk_in_ns: rec.risk_in_ns,
        me_in_ns: rec.me_in_ns,
        match_done_ns: rec.match_done_ns,
        gw_out_ns: time_ns(),
    })
    .into();
    rsx_log::latency_sample!(
        "gateway_route_serialize_done",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        latency_anchor_ns(rec)
    );
    rsx_log::latency_sample!(
        "gateway_out",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        latency_anchor_ns(rec)
    );
    let mut st = state.borrow_mut();
    st.push_to_user(rec.taker_user_id, msg.clone());
    // A self-trade has one user on both sides: one frame for the one fill.
    if rec.maker_user_id != rec.taker_user_id {
        st.push_to_user(rec.maker_user_id, msg);
    }
    rsx_log::latency_sample!(
        "gateway_route_push_done",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        latency_anchor_ns(rec)
    );
}

pub fn route_order_inserted(state: &Rc<RefCell<GatewayState>>, rec: &OrderInsertedRecord) {
    state.borrow_mut().pending.mark_resting(
        rec.user_id,
        &join_order_id(rec.order_id_hi, rec.order_id_lo),
    );
    let frame = update(
        oid_hex(rec.order_id_hi, rec.order_id_lo),
        1,
        0,
        rec.qty.0,
        0,
    );
    emit_to_user(state, rec.user_id, &frame);
}

pub fn route_order_done(state: &Rc<RefCell<GatewayState>>, rec: &OrderDoneRecord) {
    let status = rec.final_status;
    if status > 2 {
        warn!(
            "dropped ORDER_DONE with unknown final_status={status} user_id={} order_id={}",
            rec.user_id,
            oid_hex(rec.order_id_hi, rec.order_id_lo)
        );
        return;
    }
    let user_id = complete_pending(state, rec.user_id, rec.order_id_hi, rec.order_id_lo);
    let frame = WsFrame::OrderUpdate {
        order_id: oid_hex(rec.order_id_hi, rec.order_id_lo),
        status,
        filled_qty: rec.filled_qty.0,
        remaining_qty: rec.remaining_qty.0,
        reason: 0,
        gw_in_ns: rec.gw_in_ns,
        risk_in_ns: rec.risk_in_ns,
        me_in_ns: rec.me_in_ns,
        match_done_ns: rec.match_done_ns,
        gw_out_ns: time_ns(),
    };
    emit_to_user(state, user_id, &frame);
}

pub fn route_order_cancelled(state: &Rc<RefCell<GatewayState>>, rec: &OrderCancelledRecord) {
    let user_id = complete_pending(state, rec.user_id, rec.order_id_hi, rec.order_id_lo);
    // The `U` reason slot holds a CANCEL_REASON_* code under status 2 and a
    // FailureReason under status 3.
    let frame = update(
        oid_hex(rec.order_id_hi, rec.order_id_lo),
        2,
        0,
        rec.remaining_qty.0,
        rec.reason,
    );
    emit_to_user(state, user_id, &frame);
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
    state.borrow().metrics.count_reject_code(rec.reason);
    let user_id = complete_pending(state, rec.user_id, rec.order_id_hi, rec.order_id_lo);
    let frame = update(
        oid_hex(rec.order_id_hi, rec.order_id_lo),
        3,
        0,
        0,
        rec.reason,
    );
    emit_to_user(state, user_id, &frame);
}

/// Fail every order still awaiting its first response from risk that was
/// submitted before `cutoff_ns`: the order leaves the pending set and its
/// user gets a failed `U` with reason TIMEOUT, and the order counts as a
/// timeout reject. Resting orders are kept.
pub fn expire_stale_orders(state: &Rc<RefCell<GatewayState>>, cutoff_ns: u64) {
    let stale = state.borrow_mut().pending.remove_stale(cutoff_ns);
    for order in stale {
        state.borrow().metrics.count_reject(FailureReason::Timeout);
        let order_id = order_id_to_hex(&order.order_id);
        warn!(
            "order timed out user_id={} order_id={order_id}",
            order.user_id
        );
        let frame = update(order_id, 3, 0, 0, FailureReason::Timeout as u8);
        emit_to_user(state, order.user_id, &frame);
    }
}
