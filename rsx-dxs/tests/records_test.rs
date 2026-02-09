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
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: 50000,
        qty: 1000,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let encoded = encode_fill_record(1, &record);
    let header = WalHeader::from_bytes(&encoded).unwrap();
    assert_eq!(header.record_type, RECORD_FILL);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_fill_record(payload).unwrap();
    assert_eq!(decoded.seq, 42);
    assert_eq!(decoded.price, 50000);
    assert_eq!(decoded.qty, 1000);
    assert_eq!(decoded.maker_order_id_lo, 100);
    assert_eq!(decoded.taker_order_id_lo, 200);
    assert_eq!(decoded.taker_user_id, 10);
    assert_eq!(decoded.maker_user_id, 20);
}

#[test]
fn bbo_record_encode_decode_roundtrip() {
    let record = BboRecord {
        seq: 1,
        ts_ns: 2,
        symbol_id: 3,
        _pad0: 0,
        bid_px: 100,
        bid_qty: 50,
        bid_count: 5,
        _pad1: 0,
        ask_px: 101,
        ask_qty: 60,
        ask_count: 6,
        _pad2: 0,
    };
    let encoded = encode_bbo_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_bbo_record(payload).unwrap();
    assert_eq!(decoded.bid_px, 100);
    assert_eq!(decoded.ask_px, 101);
    assert_eq!(decoded.bid_count, 5);
    assert_eq!(decoded.ask_count, 6);
}

#[test]
fn order_inserted_encode_decode_roundtrip() {
    let record = OrderInsertedRecord {
        seq: 5,
        ts_ns: 10,
        symbol_id: 1,
        user_id: 42,
        order_id_hi: 0,
        order_id_lo: 999,
        price: 50000,
        qty: 100,
        side: 1,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let encoded = encode_order_inserted_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_order_inserted_record(payload).unwrap();
    assert_eq!(decoded.order_id_lo, 999);
    assert_eq!(decoded.side, 1);
    assert_eq!(decoded.user_id, 42);
}

#[test]
fn order_cancelled_encode_decode_roundtrip() {
    let record = OrderCancelledRecord {
        seq: 6,
        ts_ns: 11,
        symbol_id: 1,
        user_id: 42,
        order_id_hi: 0,
        order_id_lo: 888,
        remaining_qty: 50,
        reason: 2,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let encoded = encode_order_cancelled_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_order_cancelled_record(payload).unwrap();
    assert_eq!(decoded.order_id_lo, 888);
    assert_eq!(decoded.reason, 2);
    assert_eq!(decoded.remaining_qty, 50);
}

#[test]
fn order_done_encode_decode_roundtrip() {
    let record = OrderDoneRecord {
        seq: 7,
        ts_ns: 12,
        symbol_id: 1,
        user_id: 42,
        order_id_hi: 0,
        order_id_lo: 777,
        filled_qty: 100,
        remaining_qty: 0,
        final_status: 1,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let encoded = encode_order_done_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_order_done_record(payload).unwrap();
    assert_eq!(decoded.order_id_lo, 777);
    assert_eq!(decoded.remaining_qty, 0);
    assert_eq!(decoded.filled_qty, 100);
    assert_eq!(decoded.final_status, 1);
}

#[test]
fn config_applied_encode_decode_roundtrip() {
    let record = ConfigAppliedRecord {
        seq: 8,
        ts_ns: 13,
        symbol_id: 2,
        _pad0: 0,
        config_version: 5,
        effective_at_ms: 1000,
        applied_at_ns: 2000,
    };
    let encoded = encode_config_applied_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_config_applied_record(payload).unwrap();
    assert_eq!(decoded.config_version, 5);
    assert_eq!(decoded.effective_at_ms, 1000);
    assert_eq!(decoded.applied_at_ns, 2000);
}

#[test]
fn caught_up_encode_decode_roundtrip() {
    let record = CaughtUpRecord {
        seq: 9,
        ts_ns: 14,
        stream_id: 1,
        _pad0: 0,
        live_seq: 100,
        _pad1: [0; 40],
    };
    let encoded = encode_caught_up_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded = decode_caught_up_record(payload).unwrap();
    assert_eq!(decoded.live_seq, 100);
}

#[test]
fn order_accepted_encode_decode_roundtrip() {
    let record = OrderAcceptedRecord {
        seq: 10,
        ts_ns: 15,
        user_id: 42,
        _pad0: 0,
        order_id_hi: 0,
        order_id_lo: 555,
        _pad1: [0; 32],
    };
    let encoded = encode_order_accepted_record(1, &record);
    let payload = &encoded[WalHeader::SIZE..];
    let decoded =
        decode_order_accepted_record(payload).unwrap();
    assert_eq!(decoded.order_id_lo, 555);
    assert_eq!(decoded.user_id, 42);
}

#[test]
fn crc32_mismatch_detected() {
    let record = FillRecord {
        seq: 1,
        ts_ns: 2,
        symbol_id: 3,
        taker_user_id: 4,
        maker_user_id: 5,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 6,
        maker_order_id_hi: 0,
        maker_order_id_lo: 7,
        price: 8,
        qty: 9,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
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
        taker_user_id: 0,
        maker_user_id: 0,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: 0,
        qty: 0,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    });
    assert_eq!(record.seq(), 42);
}

#[test]
fn wal_record_type_accessor() {
    let record = WalRecord::Bbo(BboRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 0,
        _pad0: 0,
        bid_px: 0,
        bid_qty: 0,
        bid_count: 0,
        _pad1: 0,
        ask_px: 0,
        ask_qty: 0,
        ask_count: 0,
        _pad2: 0,
    });
    assert_eq!(record.record_type(), RECORD_BBO);
}

#[test]
fn cancel_reason_all_6_values_roundtrip() {
    assert_eq!(CANCEL_REASON_USER_CANCEL, 0);
    assert_eq!(CANCEL_REASON_REDUCE_ONLY, 1);
    assert_eq!(CANCEL_REASON_EXPIRY, 2);
    assert_eq!(CANCEL_REASON_SYSTEM, 3);
    assert_eq!(CANCEL_REASON_POST_ONLY_REJECT, 4);
    assert_eq!(CANCEL_REASON_OTHER, 5);
}

#[test]
fn cancel_reason_maps_to_correct_semantics() {
    let user_cancel = CANCEL_REASON_USER_CANCEL;
    let reduce_only = CANCEL_REASON_REDUCE_ONLY;
    let expiry = CANCEL_REASON_EXPIRY;
    let post_only_reject = CANCEL_REASON_POST_ONLY_REJECT;
    let system = CANCEL_REASON_SYSTEM;

    assert_eq!(user_cancel, 0);
    assert_eq!(reduce_only, 1);
    assert_eq!(expiry, 2);
    assert_eq!(system, 3);
    assert_eq!(post_only_reject, 4);
}

#[test]
fn record_truncated_header_detected() {
    let header_bytes = [0u8; 15];
    assert!(WalHeader::from_bytes(&header_bytes).is_none());
}

#[test]
fn record_truncated_payload_detected() {
    let record = FillRecord {
        seq: 1,
        ts_ns: 2,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 1,
        maker_order_id_hi: 0,
        maker_order_id_lo: 2,
        price: 100,
        qty: 10,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let payload_bytes = unsafe {
        std::slice::from_raw_parts(
            &record as *const FillRecord as *const u8,
            std::mem::size_of::<FillRecord>(),
        )
    };
    let truncated = &payload_bytes[..10];
    assert!(decode_fill_record(truncated).is_none());
}

#[test]
fn record_zero_length_payload_valid() {
    use rsx_dxs::WalHeader;
    let header = WalHeader::new(RECORD_FILL, 0, 1, 0);
    assert_eq!(header.len, 0);
}
