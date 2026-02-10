use rsx_dxs::encode_utils::as_bytes;
use rsx_dxs::encode_utils::decode_config_applied_record;
use rsx_dxs::records::ConfigAppliedRecord;
use rsx_matching::wire::EventMessage;
use rsx_matching::wire::OrderMessage;
use rsx_types::Side;
use rsx_types::TimeInForce;

#[test]
fn order_message_roundtrip_buy() {
    let msg = OrderMessage {
        seq: 1,
        price: 50_000,
        qty: 100,
        side: Side::Buy as u8,
        tif: TimeInForce::GTC as u8,
        reduce_only: 0,
        _pad1: [0; 5],
        user_id: 42,
        _pad2: 0,
        timestamp_ns: 1_000_000,
        order_id_hi: 0,
        order_id_lo: 0,
    };
    let incoming = msg.to_incoming();
    assert_eq!(incoming.price, 50_000);
    assert_eq!(incoming.qty, 100);
    assert_eq!(incoming.side, Side::Buy);
    assert_eq!(incoming.tif, TimeInForce::GTC);
    assert!(!incoming.reduce_only);
    assert_eq!(incoming.user_id, 42);
}

#[test]
fn order_message_roundtrip_sell_ioc() {
    let msg = OrderMessage {
        seq: 2,
        price: 49_000,
        qty: 200,
        side: Side::Sell as u8,
        tif: TimeInForce::IOC as u8,
        reduce_only: 1,
        _pad1: [0; 5],
        user_id: 7,
        _pad2: 0,
        timestamp_ns: 999,
        order_id_hi: 0,
        order_id_lo: 0,
    };
    let incoming = msg.to_incoming();
    assert_eq!(incoming.side, Side::Sell);
    assert_eq!(incoming.tif, TimeInForce::IOC);
    assert!(incoming.reduce_only);
}

#[test]
fn event_message_from_fill() {
    let event = rsx_book::event::Event::Fill {
        maker_handle: 1,
        maker_user_id: 1,
        taker_user_id: 2,
        price: rsx_types::Price(50_000),
        qty: rsx_types::Qty(100),
        side: Side::Buy as u8,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
    };
    let msg = EventMessage::from_book_event(&event);
    match msg {
        EventMessage::Fill {
            maker_handle,
            taker_user_id,
            price,
            qty,
            side,
            ..
        } => {
            assert_eq!(maker_handle, 1);
            assert_eq!(taker_user_id, 2);
            assert_eq!(price, 50_000);
            assert_eq!(qty, 100);
            assert_eq!(side, Side::Buy as u8);
        }
        _ => panic!("expected fill"),
    }
}

#[test]
fn event_message_from_order_inserted() {
    let event = rsx_book::event::Event::OrderInserted {
        handle: 5,
        user_id: 3,
        side: Side::Sell as u8,
        price: rsx_types::Price(49_000),
        qty: rsx_types::Qty(200),
        order_id_hi: 0,
        order_id_lo: 0,
    };
    let msg = EventMessage::from_book_event(&event);
    assert!(matches!(
        msg,
        EventMessage::OrderInserted { handle: 5, .. }
    ));
}

#[test]
fn config_applied_record_roundtrip() {
    let record = ConfigAppliedRecord {
        seq: 42,
        ts_ns: 1_000_000_000,
        symbol_id: 7,
        _pad0: 0,
        config_version: 3,
        effective_at_ms: 500,
        applied_at_ns: 1_000_000_000,
    };
    let bytes = as_bytes(&record);
    let decoded =
        decode_config_applied_record(bytes).unwrap();
    assert_eq!(decoded.seq, 42);
    assert_eq!(decoded.symbol_id, 7);
    assert_eq!(decoded.config_version, 3);
    assert_eq!(decoded.effective_at_ms, 500);
    assert_eq!(decoded.applied_at_ns, 1_000_000_000);
}
