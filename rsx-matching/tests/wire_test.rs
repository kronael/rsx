use rsx_cast::encode_utils::as_bytes;
use rsx_matching::wire::decode_side;
use rsx_matching::wire::decode_tif;
use rsx_matching::wire::to_incoming;
use rsx_messages::decode_config_applied_record;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::OrderMessage;
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
        post_only: 0,
        _pad1: [0; 4],
        user_id: 42,
        _pad2: 0,
        timestamp_ns: 1_000_000,
        order_id_hi: 0,
        order_id_lo: 0,
        risk_in_ns: 123,
        _pad3: [0; 56],
    };
    let incoming = to_incoming(&msg);
    assert_eq!(incoming.price, 50_000);
    assert_eq!(incoming.qty, 100);
    assert_eq!(incoming.side, Side::Buy);
    assert_eq!(incoming.tif, TimeInForce::GTC);
    assert!(!incoming.reduce_only);
    assert_eq!(incoming.user_id, 42);
    assert_eq!(std::mem::size_of::<OrderMessage>(), 128);
    assert_eq!(std::mem::align_of::<OrderMessage>(), 64);
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
        post_only: 1,
        _pad1: [0; 4],
        user_id: 7,
        _pad2: 0,
        timestamp_ns: 999,
        order_id_hi: 0,
        order_id_lo: 0,
        risk_in_ns: 456,
        _pad3: [0; 56],
    };
    let incoming = to_incoming(&msg);
    assert_eq!(incoming.side, Side::Sell);
    assert_eq!(incoming.tif, TimeInForce::IOC);
    assert!(incoming.reduce_only);
    assert!(incoming.post_only);
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
        _pad1: [0; 16],
    };
    let bytes = as_bytes(&record);
    let decoded = decode_config_applied_record(bytes).unwrap();
    assert_eq!(decoded.seq, 42);
    assert_eq!(decoded.symbol_id, 7);
    assert_eq!(decoded.config_version, 3);
    assert_eq!(decoded.effective_at_ms, 500);
    assert_eq!(decoded.applied_at_ns, 1_000_000_000);
}

#[test]
fn decode_side_and_tif_cover_wire_values() {
    assert_eq!(decode_side(Side::Buy as u8), Side::Buy);
    assert_eq!(decode_side(Side::Sell as u8), Side::Sell);
    assert_eq!(decode_tif(TimeInForce::GTC as u8), TimeInForce::GTC);
    assert_eq!(decode_tif(TimeInForce::IOC as u8), TimeInForce::IOC);
    assert_eq!(decode_tif(TimeInForce::FOK as u8), TimeInForce::FOK);
    // Out-of-range bytes decode totally: Sell and GTC.
    assert_eq!(decode_side(2), Side::Sell);
    assert_eq!(decode_tif(3), TimeInForce::GTC);
}
