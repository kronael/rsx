use rsx_dxs::WalHeader;

#[test]
fn header_encode_decode_roundtrip() {
    let header = WalHeader::new(1, 64, 0xDEADBEEF);
    let bytes = header.to_bytes();
    let decoded = WalHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.record_type, 1);
    assert_eq!(decoded.len, 64);
    assert_eq!(decoded.crc32, 0xDEADBEEF);
    assert_eq!(decoded._reserved, [0u8; 8]);
}

#[test]
fn header_little_endian_verified() {
    // Test from_bytes directly with known byte layout
    let raw: [u8; 16] = [
        0x02, 0x01, // record_type = 0x0102 LE
        0x03, 0x04, // len = 0x0403 LE
        0x05, 0x06, 0x07, 0x08, // crc32 LE
        0x00, 0x00, 0x00, 0x00, // _reserved
        0x00, 0x00, 0x00, 0x00, // _reserved
    ];
    let h = WalHeader::from_bytes(&raw).unwrap();
    assert_eq!(h.record_type, 0x0102);
    assert_eq!(h.len, 0x0403);
    assert_eq!(h.crc32, 0x08070605);
    assert_eq!(h._reserved, [0u8; 8]);
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

#[test]
fn wal_header_crc32_matches_payload() {
    use rsx_dxs::compute_crc32;
    let payload = b"test payload data";
    let crc = compute_crc32(payload);
    let header = WalHeader::new(1, payload.len() as u16, crc);
    assert_eq!(header.crc32, crc);
}
