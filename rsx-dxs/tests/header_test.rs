use rsx_dxs::WalHeader;

#[test]
fn header_encode_decode_roundtrip() {
    let header = WalHeader::new(1, 64, 42, 0xDEADBEEF);
    let bytes = header.to_bytes();
    let decoded = WalHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.version, WalHeader::VERSION);
    assert_eq!(decoded.record_type, 1);
    assert_eq!(decoded.len, 64);
    assert_eq!(decoded.stream_id, 42);
    assert_eq!(decoded.crc32, 0xDEADBEEF);
}

#[test]
fn header_little_endian_verified() {
    let header = WalHeader::new(0x0102, 0x03040506, 0x0708090A, 0x0B0C0D0E);
    let bytes = header.to_bytes();
    // version: 0x0102 LE => [0x02, 0x01]
    assert_eq!(bytes[0], 0x01); // VERSION constant is 1
    // record_type gets overridden by VERSION in new()
    // Let's test from_bytes directly
    let raw: [u8; 16] = [
        0x02, 0x01, // version = 0x0102 LE
        0x03, 0x04, // record_type = 0x0403 LE
        0x05, 0x06, 0x07, 0x08, // len LE
        0x09, 0x0A, 0x0B, 0x0C, // stream_id LE
        0x0D, 0x0E, 0x0F, 0x10, // crc32 LE
    ];
    let h = WalHeader::from_bytes(&raw).unwrap();
    assert_eq!(h.version, 0x0102);
    assert_eq!(h.record_type, 0x0403);
    assert_eq!(h.len, 0x08070605);
    assert_eq!(h.stream_id, 0x0C0B0A09);
    assert_eq!(h.crc32, 0x100F0E0D);
}

#[test]
fn header_from_bytes_too_short_returns_none() {
    let buf = [0u8; 15];
    assert!(WalHeader::from_bytes(&buf).is_none());
}

#[test]
fn header_size_is_16() {
    assert_eq!(WalHeader::SIZE, 16);
}
