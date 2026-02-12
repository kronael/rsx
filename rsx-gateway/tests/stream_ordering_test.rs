use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderFailedRecord;
use rsx_gateway::order_id::order_id_to_hex;
use rsx_gateway::pending::PendingOrder;
use rsx_gateway::protocol::parse;
use rsx_gateway::protocol::WsFrame;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_failed;
use rsx_gateway::state::GatewayState;
use rsx_types::Price;
use rsx_types::Qty;
use std::cell::RefCell;
use std::rc::Rc;

fn oid_bytes(hi: u64, lo: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes
}

fn setup() -> (Rc<RefCell<GatewayState>>, u64) {
    let state = Rc::new(RefCell::new(
        GatewayState::new(10, 10, 30_000, vec![]),
    ));
    let conn =
        state.borrow_mut().add_connection(7).unwrap();
    (state, conn)
}

fn add_pending(
    state: &Rc<RefCell<GatewayState>>,
    oid: [u8; 16],
) {
    let pending = PendingOrder {
        order_id: oid,
        user_id: 7,
        symbol_id: 1,
        client_order_id: [b'0'; 20],
        timestamp_ns: 10,
    };
    assert!(state.borrow_mut().pending.push(pending));
}

#[test]
fn fills_precede_order_done_in_stream() {
    let (state, conn) = setup();
    let hi = 0xAAAA_BBBB_CCCC_DDDDu64;
    let lo = 0x1111_2222_3333_4444u64;
    let oid = oid_bytes(hi, lo);
    add_pending(&state, oid);

    // Route fill first, then done
    let fill = FillRecord {
        seq: 1,
        ts_ns: 1,
        symbol_id: 1,
        taker_user_id: 7,
        maker_user_id: 8,
        _pad0: 0,
        taker_order_id_hi: hi,
        taker_order_id_lo: lo,
        maker_order_id_hi: hi,
        maker_order_id_lo: lo,
        price: Price(100),
        qty: Qty(5),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_fill(&state, &fill);

    let done = OrderDoneRecord {
        seq: 2,
        ts_ns: 2,
        symbol_id: 1,
        user_id: 7,
        order_id_hi: hi,
        order_id_lo: lo,
        filled_qty: Qty(5),
        remaining_qty: Qty(0),
        final_status: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_order_done(&state, &done);

    let msgs = state.borrow_mut().drain_outbound(conn);
    assert_eq!(msgs.len(), 2);

    // First message is Fill, second is OrderDone
    let first = parse(&msgs[0]).unwrap();
    let second = parse(&msgs[1]).unwrap();
    assert!(
        matches!(first, WsFrame::Fill { .. }),
        "first message must be Fill, got {:?}",
        first,
    );
    assert!(
        matches!(second, WsFrame::OrderUpdate { .. }),
        "second message must be OrderUpdate, got {:?}",
        second,
    );
}

#[test]
fn exactly_one_completion_per_order() {
    let (state, conn) = setup();
    let hi = 0xDDDD_EEEE_FFFF_0000u64;
    let lo = 0x5555_6666_7777_8888u64;
    let oid = oid_bytes(hi, lo);
    let oid_hex = order_id_to_hex(&oid);
    add_pending(&state, oid);

    let done = OrderDoneRecord {
        seq: 1,
        ts_ns: 1,
        symbol_id: 1,
        user_id: 7,
        order_id_hi: hi,
        order_id_lo: lo,
        filled_qty: Qty(5),
        remaining_qty: Qty(0),
        final_status: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    route_order_done(&state, &done);

    let msgs = state.borrow_mut().drain_outbound(conn);
    assert_eq!(msgs.len(), 1);

    match parse(&msgs[0]).unwrap() {
        WsFrame::OrderUpdate {
            order_id,
            status,
            ..
        } => {
            assert_eq!(order_id, oid_hex);
            assert_eq!(status, 0); // filled
        }
        other => panic!(
            "expected OrderUpdate, got {:?}",
            other,
        ),
    }

    // Pending cleared after done
    assert!(state.borrow().pending.is_empty());
}

#[test]
fn order_done_or_failed_never_both() {
    let (state, conn) = setup();
    let hi = 0x1234_5678_9ABC_DEF0u64;
    let lo = 0xFEDC_BA98_7654_3210u64;
    let oid = oid_bytes(hi, lo);
    let oid_hex = order_id_to_hex(&oid);
    add_pending(&state, oid);

    // Route ORDER_FAILED
    let failed = OrderFailedRecord {
        seq: 1,
        ts_ns: 1,
        user_id: 7,
        _pad0: 0,
        order_id_hi: hi,
        order_id_lo: lo,
        reason: 1,
        _pad: [0; 23],
    };
    route_order_failed(&state, &failed);

    let msgs = state.borrow_mut().drain_outbound(conn);
    assert_eq!(msgs.len(), 1);

    match parse(&msgs[0]).unwrap() {
        WsFrame::OrderUpdate {
            order_id,
            status,
            reason,
            ..
        } => {
            assert_eq!(order_id, oid_hex);
            assert_eq!(status, 3); // failed
            assert_eq!(reason, 1);
        }
        other => panic!(
            "expected OrderUpdate, got {:?}",
            other,
        ),
    }

    // Pending cleared
    assert!(state.borrow().pending.is_empty());

    // No further messages (done never sent)
    let msgs = state.borrow_mut().drain_outbound(conn);
    assert!(msgs.is_empty());
}
