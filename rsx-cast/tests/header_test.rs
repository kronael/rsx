use rsx_cast::WalHeader;
use rsx_cast::WalVersion;

#[test]
fn header_encode_decode_roundtrip() {
    let header = WalHeader::new(1, 64, 0xDEADBEEF);
    let bytes = header.to_bytes();
    let decoded = WalHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.version, WalVersion::V1 as u8);
    assert_eq!(decoded.record_type, 1);
    assert_eq!(decoded.len, 64);
    assert_eq!(decoded.crc32, 0xDEADBEEF);
}

#[test]
fn header_little_endian_verified() {
    let raw: [u8; 16] = [
        WalVersion::V1 as u8, // version
        0x00,                 // _pad0
        0x02, 0x01,           // record_type = 0x0102 LE
        0x03, 0x04,           // len = 0x0403 LE
        0x00, 0x00,           // _pad1
        0x05, 0x06, 0x07, 0x08, // crc32 LE
        0x00, 0x00, 0x00, 0x00, // _reserved
    ];
    let h = WalHeader::from_bytes(&raw).unwrap();
    assert_eq!(h.version, WalVersion::V1 as u8);
    assert_eq!(h.record_type, 0x0102);
    assert_eq!(h.len, 0x0403);
    assert_eq!(h.crc32, 0x08070605);
}

#[test]
fn header_new_writes_v1() {
    let header = WalHeader::new(1, 0, 0);
    assert_eq!(header.version, WalVersion::V1 as u8);
}

#[test]
fn header_unknown_version_returns_none() {
    for bad in [0x00u8, 0xFF] {
        let mut raw = [0u8; 16];
        raw[0] = bad;
        raw[2] = 0x01;
        assert!(
            WalHeader::from_bytes(&raw).is_none(),
            "version {bad:#x} should be rejected"
        );
    }
}

#[test]
fn header_from_bytes_too_short_returns_none() {
    assert!(WalHeader::from_bytes(&[0u8; 15]).is_none());
}

#[test]
fn header_size_is_16() {
    assert_eq!(WalHeader::SIZE, 16);
}

#[test]
fn wal_header_crc32_matches_payload() {
    use rsx_cast::compute_crc32;
    let payload = b"test payload data";
    let crc = compute_crc32(payload);
    let header = WalHeader::new(1, payload.len() as u16, crc);
    assert_eq!(header.crc32, crc);
}

#[test]
fn header_from_bytes_no_panic_on_random_input() {
    let mut state: u64 = 0xCAFEBABEu64;
    let mut buf = [0u8; 16];
    for _ in 0..100_000 {
        for b in buf.iter_mut() {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            *b = (z ^ (z >> 31)) as u8;
        }
        let _ = WalHeader::from_bytes(&buf);
    }
}

#[test]
fn header_from_bytes_no_panic_on_short_input() {
    for len in 0..16 {
        assert!(WalHeader::from_bytes(&vec![0xFFu8; len]).is_none());
    }
}
