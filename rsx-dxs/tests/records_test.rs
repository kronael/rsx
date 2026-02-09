use rsx_dxs::*;
use std::mem;

#[test]
fn fill_record_is_64_aligned() {
    assert_eq!(mem::align_of::<FillRecord>(), 64);
}

#[test]
fn bbo_record_is_64_aligned() {
    assert_eq!(mem::align_of::<BboRecord>(), 64);
}

#[test]
fn order_inserted_record_is_64_aligned() {
    assert_eq!(mem::align_of::<OrderInsertedRecord>(), 64);
}

#[test]
fn order_cancelled_record_is_64_aligned() {
    assert_eq!(mem::align_of::<OrderCancelledRecord>(), 64);
}

#[test]
fn order_done_record_is_64_aligned() {
    assert_eq!(mem::align_of::<OrderDoneRecord>(), 64);
}

#[test]
fn config_applied_record_is_64_aligned() {
    assert_eq!(mem::align_of::<ConfigAppliedRecord>(), 64);
}

#[test]
fn caught_up_record_is_64_aligned() {
    assert_eq!(mem::align_of::<CaughtUpRecord>(), 64);
}

#[test]
fn order_accepted_record_is_64_aligned() {
    assert_eq!(mem::align_of::<OrderAcceptedRecord>(), 64);
}

#[test]
fn fill_record_encode_decode_roundtrip() {
    let record = FillRecord {
        seq: 42,
        ts_ns: 1_000_000_000,
        symbol_id: 1,
        maker_oid: 100,
        taker_oid: 200,
        px: 50000,
        qty: 1000,
        maker_side: 0,
        _pad1: [0; 7],
    };
    let encoded = encode_fill_record(1, &record);
    let header = WalHeader::from_bytes(&encoded).unwrap();
    assert_eq!(header.record_type, RECORD_FILL);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_fill_record(payload).unwrap();
    assert_eq!(decoded.seq, 42);
    assert_eq!(decoded.px, 50000);
    assert_eq!(decoded.qty, 1000);
    assert_eq!(decoded.maker_oid, 100);
    assert_eq!(decoded.taker_oid, 200);
}

#[test]
fn bbo_record_encode_decode_roundtrip() {
    let record = BboRecord {
        seq: 1,
        ts_ns: 2,
        symbol_id: 3,
        bid_px: 100,
        ask_px: 101,
        bid_qty: 50,
        ask_qty: 60,
        _pad1: [0; 4],
    };
    let encoded = encode_bbo_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_bbo_record(payload).unwrap();
    assert_eq!(decoded.bid_px, 100);
    assert_eq!(decoded.ask_px, 101);
}

#[test]
fn order_inserted_encode_decode_roundtrip() {
    let record = OrderInsertedRecord {
        seq: 5,
        ts_ns: 10,
        symbol_id: 1,
        oid: 999,
        user_id: 42,
        px: 50000,
        qty: 100,
        side: 1,
        _pad1: [0; 7],
    };
    let encoded = encode_order_inserted_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_order_inserted_record(payload).unwrap();
    assert_eq!(decoded.oid, 999);
    assert_eq!(decoded.side, 1);
}

#[test]
fn order_cancelled_encode_decode_roundtrip() {
    let record = OrderCancelledRecord {
        seq: 6,
        ts_ns: 11,
        symbol_id: 1,
        oid: 888,
        reason: 2,
        _pad1: [0; 7],
    };
    let encoded = encode_order_cancelled_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_order_cancelled_record(payload).unwrap();
    assert_eq!(decoded.oid, 888);
    assert_eq!(decoded.reason, 2);
}

#[test]
fn order_done_encode_decode_roundtrip() {
    let record = OrderDoneRecord {
        seq: 7,
        ts_ns: 12,
        symbol_id: 1,
        oid: 777,
        remaining_qty: 0,
        reason: 1,
        _pad1: [0; 7],
    };
    let encoded = encode_order_done_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_order_done_record(payload).unwrap();
    assert_eq!(decoded.oid, 777);
    assert_eq!(decoded.remaining_qty, 0);
}

#[test]
fn config_applied_encode_decode_roundtrip() {
    let record = ConfigAppliedRecord {
        seq: 8,
        ts_ns: 13,
        symbol_id: 2,
        config_version: 5,
        _pad1: [0; 40],
    };
    let encoded = encode_config_applied_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_config_applied_record(payload).unwrap();
    assert_eq!(decoded.config_version, 5);
}

#[test]
fn caught_up_encode_decode_roundtrip() {
    let record = CaughtUpRecord {
        seq: 9,
        ts_ns: 14,
        stream_id: 1,
        live_seq: 100,
        _pad1: [0; 36],
    };
    let encoded = encode_caught_up_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_caught_up_record(payload).unwrap();
    assert_eq!(decoded.live_seq, 100);
}

#[test]
fn order_accepted_encode_decode_roundtrip() {
    let mut cid = [0u8; 20];
    cid[..5].copy_from_slice(b"test1");
    let record = OrderAcceptedRecord {
        seq: 10,
        ts_ns: 15,
        symbol_id: 1,
        oid: 555,
        cid,
        user_id: 42,
        _pad1: [0; 20],
    };
    let encoded = encode_order_accepted_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_order_accepted_record(payload).unwrap();
    assert_eq!(decoded.oid, 555);
    assert_eq!(&decoded.cid[..5], b"test1");
}

#[test]
fn crc32_mismatch_detected() {
    let record = FillRecord {
        seq: 1,
        ts_ns: 2,
        symbol_id: 3,
        maker_oid: 4,
        taker_oid: 5,
        px: 6,
        qty: 7,
        maker_side: 0,
        _pad1: [0; 7],
    };
    let mut encoded = encode_fill_record(1, &record);
    // corrupt a payload byte
    encoded[WalHeader::SIZE] ^= 0xFF;
    let header = WalHeader::from_bytes(&encoded).unwrap();
    let payload = &encoded[WalHeader::SIZE..];
    let computed = compute_crc32(payload);
    assert_ne!(computed, header.crc32);
}

#[test]
fn wal_record_seq_accessor() {
    let record = WalRecord::Fill(FillRecord {
        seq: 42,
        ts_ns: 0,
        symbol_id: 0,
        maker_oid: 0,
        taker_oid: 0,
        px: 0,
        qty: 0,
        maker_side: 0,
        _pad1: [0; 7],
    });
    assert_eq!(record.seq(), 42);
}

#[test]
fn wal_record_type_accessor() {
    let record = WalRecord::Bbo(BboRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 0,
        bid_px: 0,
        ask_px: 0,
        bid_qty: 0,
        ask_qty: 0,
        _pad1: [0; 4],
    });
    assert_eq!(record.record_type(), RECORD_BBO);
}
