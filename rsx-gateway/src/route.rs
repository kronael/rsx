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

fn oid_bytes(hi: u64, lo: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes
}

fn oid_hex(hi: u64, lo: u64) -> String {
    order_id_to_hex(&oid_bytes(hi, lo))
}

pub fn route_fill(
    state: &Rc<RefCell<GatewayState>>,
    rec: &FillRecord,
) {
    let taker_oid =
        oid_hex(rec.taker_order_id_hi, rec.taker_order_id_lo);
    let maker_oid =
        oid_hex(rec.maker_order_id_hi, rec.maker_order_id_lo);
    let msg = serialize(&WsFrame::Fill {
        taker_order_id: taker_oid.clone(),
        maker_order_id: maker_oid.clone(),
        price: rec.price.0,
        qty: rec.qty.0,
        timestamp_ns: rec.ts_ns,
        fee: 0, // v1: fee not in FillRecord, computed at risk layer
    });
    // F4.3 — per-stage latency trace. Stage `gateway_out`
    // closes the GW→ME→GW loop. Anchor t_us against the
    // taker's gateway-ingress timestamp (rec.taker_ts_ns)
    // so the closing delta composes with gateway_in /
    // risk_in / me_in / me_out / risk_out on the same
    // clock origin. Falls back to rec.ts_ns (ME emit) when
    // the field is missing (legacy WAL records).
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let anchor_ns = if rec.taker_ts_ns == 0 {
        rec.ts_ns
    } else {
        rec.taker_ts_ns
    };
    let t_us = now_ns.saturating_sub(anchor_ns) / 1000;
    // Sub-stage: serialize completed. Captured BEFORE the
    // gateway_out emission so we can attribute the serde_json
    // cost separately from the latency-ring emission itself.
    rsx_log::latency::sample(
        "gateway_route_serialize_done",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        t_us,
        anchor_ns,
    );
    rsx_log::latency::sample(
        "gateway_out",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        t_us,
        anchor_ns,
    );
    let mut st = state.borrow_mut();
    st.push_to_user(rec.taker_user_id, msg.clone());
    st.push_to_user(rec.maker_user_id, msg);
    drop(st);
    // Sub-stage: both push_to_user calls completed (both
    // outbound VecDeques now hold the fill frame).
    let now_ns2 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let t_us2 = now_ns2.saturating_sub(anchor_ns) / 1000;
    rsx_log::latency::sample(
        "gateway_route_push_done",
        rec.taker_order_id_hi,
        rec.taker_order_id_lo,
        t_us2,
        anchor_ns,
    );
}

pub fn route_order_inserted(
    state: &Rc<RefCell<GatewayState>>,
    rec: &OrderInsertedRecord,
) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    let msg = serialize(&WsFrame::OrderUpdate {
        order_id: oid,
        status: 1, // resting/accepted from matching
        filled_qty: 0,
        remaining_qty: rec.qty.0,
        reason: 0,
    });
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
}

pub fn route_order_done(
    state: &Rc<RefCell<GatewayState>>,
    rec: &OrderDoneRecord,
) {
    let status = match rec.final_status {
        0 => 0, // filled
        1 => 1, // resting (unexpected for done)
        2 => 2, // cancelled
        _ => 0,
    };
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    let msg = serialize(&WsFrame::OrderUpdate {
        order_id: oid,
        status,
        filled_qty: rec.filled_qty.0,
        remaining_qty: rec.remaining_qty.0,
        reason: 0,
    });
    let oid_bytes_val =
        oid_bytes(rec.order_id_hi, rec.order_id_lo);
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
    st.pending.remove(&oid_bytes_val);
}

pub fn route_order_cancelled(
    state: &Rc<RefCell<GatewayState>>,
    rec: &OrderCancelledRecord,
) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    let msg = serialize(&WsFrame::OrderUpdate {
        order_id: oid,
        status: 2, // cancelled
        filled_qty: 0,
        remaining_qty: rec.remaining_qty.0,
        reason: 0,
    });
    let oid_bytes_val =
        oid_bytes(rec.order_id_hi, rec.order_id_lo);
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
    st.pending.remove(&oid_bytes_val);
}

pub fn route_liquidation(
    state: &Rc<RefCell<GatewayState>>,
    rec: &LiquidationRecord,
) {
    let msg = serialize(&WsFrame::Liquidation {
        symbol_id: rec.symbol_id,
        status: rec.status,
        round: rec.round,
        side: rec.side,
        qty: rec.qty,
        price: rec.price,
        slip_bps: rec.slip_bps,
    });
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
}

pub fn route_order_failed(
    state: &Rc<RefCell<GatewayState>>,
    rec: &OrderFailedRecord,
) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    let msg = serialize(&WsFrame::OrderUpdate {
        order_id: oid,
        status: 3, // failed
        filled_qty: 0,
        remaining_qty: 0,
        reason: rec.reason,
    });
    let oid_bytes_val =
        oid_bytes(rec.order_id_hi, rec.order_id_lo);
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
    st.pending.remove(&oid_bytes_val);
}
