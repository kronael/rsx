use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_gateway::order_id::order_id_to_hex;
use rsx_gateway::pending::PendingOrder;
use rsx_gateway::protocol::serialize;
use rsx_gateway::protocol::WsFrame;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_order_cancelled;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_inserted;
use rsx_gateway::state::GatewayState;
use std::cell::RefCell;
use std::rc::Rc;

fn oid_bytes(hi: u64, lo: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes
}

#[test]
fn order_lifecycle_fill_done_routes_to_user() {
    let state = Rc::new(RefCell::new(GatewayState::new(
        10, 10, 30_000, vec![],
    )));
    let conn_taker = state.borrow_mut().add_connection(7);
    let conn_maker = state.borrow_mut().add_connection(8);

    let oid_hi = 0x0102_0304_0506_0708u64;
    let oid_lo = 0x090A_0B0C_0D0E_0F10u64;
    let oid = oid_bytes(oid_hi, oid_lo);
    let oid_hex = order_id_to_hex(&oid);

    let pending = PendingOrder {
        order_id: oid,
        user_id: 7,
        symbol_id: 1,
        client_order_id: [b'0'; 20],
        timestamp_ns: 10,
    };
    assert!(state.borrow_mut().pending.push(pending));

    let inserted = OrderInsertedRecord {
        seq: 1,
        ts_ns: 1,
        symbol_id: 1,
        user_id: 7,
        order_id_hi: oid_hi,
        order_id_lo: oid_lo,
        price: 100,
        qty: 5,
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_order_inserted(&state, &inserted);

    let expected_inserted = serialize(&WsFrame::OrderUpdate {
        order_id: oid_hex.clone(),
        status: 1,
        filled_qty: 0,
        remaining_qty: 5,
        reason: 0,
    });
    let out = state.borrow_mut().drain_outbound(conn_taker);
    assert_eq!(out, vec![expected_inserted]);

    let fill = FillRecord {
        seq: 2,
        ts_ns: 2,
        symbol_id: 1,
        taker_user_id: 7,
        maker_user_id: 8,
        _pad0: 0,
        taker_order_id_hi: oid_hi,
        taker_order_id_lo: oid_lo,
        maker_order_id_hi: oid_hi,
        maker_order_id_lo: oid_lo,
        price: 100,
        qty: 2,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_fill(&state, &fill);

    let expected_fill = serialize(&WsFrame::Fill {
        taker_order_id: oid_hex.clone(),
        maker_order_id: oid_hex.clone(),
        price: 100,
        qty: 2,
        timestamp_ns: 2,
        fee: 0,
    });
    let out_taker = state.borrow_mut().drain_outbound(conn_taker);
    let out_maker = state.borrow_mut().drain_outbound(conn_maker);
    assert_eq!(out_taker, vec![expected_fill.clone()]);
    assert_eq!(out_maker, vec![expected_fill]);

    let done = OrderDoneRecord {
        seq: 3,
        ts_ns: 3,
        symbol_id: 1,
        user_id: 7,
        order_id_hi: oid_hi,
        order_id_lo: oid_lo,
        filled_qty: 5,
        remaining_qty: 0,
        final_status: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_order_done(&state, &done);

    let expected_done = serialize(&WsFrame::OrderUpdate {
        order_id: oid_hex.clone(),
        status: 0,
        filled_qty: 5,
        remaining_qty: 0,
        reason: 0,
    });
    let out = state.borrow_mut().drain_outbound(conn_taker);
    assert_eq!(out, vec![expected_done]);
    assert!(state.borrow().pending.is_empty());
}

#[test]
fn order_cancel_routes_and_clears_pending() {
    let state = Rc::new(RefCell::new(GatewayState::new(
        10, 10, 30_000, vec![],
    )));
    let conn = state.borrow_mut().add_connection(7);

    let oid_hi = 0x1111_1111_1111_1111u64;
    let oid_lo = 0x2222_2222_2222_2222u64;
    let oid = oid_bytes(oid_hi, oid_lo);
    let oid_hex = order_id_to_hex(&oid);

    let pending = PendingOrder {
        order_id: oid,
        user_id: 7,
        symbol_id: 1,
        client_order_id: [b'1'; 20],
        timestamp_ns: 10,
    };
    assert!(state.borrow_mut().pending.push(pending));

    let cancelled = OrderCancelledRecord {
        seq: 1,
        ts_ns: 1,
        symbol_id: 1,
        user_id: 7,
        order_id_hi: oid_hi,
        order_id_lo: oid_lo,
        remaining_qty: 3,
        reason: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_order_cancelled(&state, &cancelled);

    let expected = serialize(&WsFrame::OrderUpdate {
        order_id: oid_hex,
        status: 2,
        filled_qty: 0,
        remaining_qty: 3,
        reason: 0,
    });
    let out = state.borrow_mut().drain_outbound(conn);
    assert_eq!(out, vec![expected]);
    assert!(state.borrow().pending.is_empty());
}
