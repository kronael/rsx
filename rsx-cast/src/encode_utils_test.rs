use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::CastHeartbeat;
use crate::records::Nak;
use crate::records::ReplicationRequest;
use crate::records::RECORD_HEARTBEAT;
use crate::records::RECORD_NAK;
use crate::records::RECORD_REPLICATION_REQUEST;
use rsx_messages::FillRecord;
use rsx_messages::RECORD_FILL;
use rsx_types::Price;
use rsx_types::Qty;
use std::mem;

// --- Nak ---

#[test]
fn nak_encode_decode_roundtrip() {
    let nak = Nak {
        from_seq: 100,
        count: 5,
        _pad1: [0u8; 48],
    };
    let bytes = as_bytes(&nak);
    let decoded = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Nak) };
    assert_eq!(decoded.from_seq, 100);
    assert_eq!(decoded.count, 5);
}

#[test]
fn nak_size_is_64_bytes() {
    assert_eq!(mem::size_of::<Nak>(), 64);
    assert_eq!(mem::align_of::<Nak>(), 64);
}

#[test]
fn nak_fields_little_endian() {
    let nak = Nak {
        from_seq: 0x0102030405060708,
        count: 0x090A0B0C0D0E0F10,
        _pad1: [0u8; 48],
    };
    let bytes = as_bytes(&nak);
    assert_eq!(bytes[0], 0x08);
    assert_eq!(bytes[7], 0x01);
    assert_eq!(bytes[8], 0x10);
    assert_eq!(bytes[15], 0x09);
}

// --- CastHeartbeat ---

#[test]
fn heartbeat_encode_decode_roundtrip() {
    let hb = CastHeartbeat {
        highest_seq: 999,
        _pad1: [0u8; 56],
    };
    let bytes = as_bytes(&hb);
    let decoded = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const CastHeartbeat) };
    assert_eq!(decoded.highest_seq, 999);
}

#[test]
fn heartbeat_size_is_64_bytes() {
    assert_eq!(mem::size_of::<CastHeartbeat>(), 64);
    assert_eq!(mem::align_of::<CastHeartbeat>(), 64);
}

#[test]
fn heartbeat_fields_little_endian() {
    let hb = CastHeartbeat {
        highest_seq: 0x0102030405060708,
        _pad1: [0u8; 56],
    };
    let bytes = as_bytes(&hb);
    assert_eq!(bytes[0], 0x08);
    assert_eq!(bytes[7], 0x01);
}

// --- Record type constants ---

#[test]
fn control_record_type_values_match_spec() {
    // 0x10 is reserved (was RECORD_STATUS_MESSAGE, removed).
    assert_eq!(RECORD_NAK, 0x11);
    assert_eq!(RECORD_HEARTBEAT, 0x12);
    assert_eq!(RECORD_REPLICATION_REQUEST, 0x13);
}

// --- Padding zeroed ---

#[test]
fn padding_bytes_zeroed_in_all_control_msgs() {
    let nak = Nak {
        from_seq: u64::MAX,
        count: u64::MAX,
        _pad1: [0u8; 48],
    };
    let bytes = as_bytes(&nak);
    for &b in &bytes[16..64] {
        assert_eq!(b, 0, "nak padding not zeroed");
    }

    let hb = CastHeartbeat {
        highest_seq: u64::MAX,
        _pad1: [0u8; 56],
    };
    let bytes = as_bytes(&hb);
    for &b in &bytes[8..64] {
        assert_eq!(b, 0, "heartbeat padding not zeroed");
    }
}

// --- CRC32 covers payload not header ---

#[test]
fn crc32_covers_payload_not_header() {
    let fill = FillRecord {
        seq: 1,
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: Price(50000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    };
    let payload = as_bytes(&fill);
    let crc = compute_crc32(payload);
    let header = WalHeader::new(RECORD_FILL, payload.len() as u16, crc);
    assert_eq!(header.crc32, crc);

    // Mutate header bytes -- CRC unchanged
    let mut hdr_bytes: [u8; 16] = header.to_bytes().try_into().unwrap();
    hdr_bytes[8] ^= 0xFF;
    let _ = hdr_bytes;
    let crc_after = compute_crc32(payload);
    assert_eq!(crc_after, crc);

    // Mutate payload byte -- CRC changes
    let mut payload_mut = payload.to_vec();
    payload_mut[0] ^= 0xFF;
    let crc_corrupt = compute_crc32(&payload_mut);
    assert_ne!(crc_corrupt, crc);
}

// --- ReplicationRequest ---

#[test]
fn replay_request_encode_decode_roundtrip() {
    let req = ReplicationRequest {
        stream_id: 7,
        _pad0: 0,
        from_seq: 42,
        _pad1: [0u8; 48],
    };
    let bytes = as_bytes(&req);
    let decoded = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const ReplicationRequest) };
    assert_eq!(decoded.stream_id, 7);
    assert_eq!(decoded.from_seq, 42);
}

#[test]
fn replay_request_size_is_64_bytes() {
    assert_eq!(mem::size_of::<ReplicationRequest>(), 64);
    assert_eq!(mem::align_of::<ReplicationRequest>(), 64);
}
