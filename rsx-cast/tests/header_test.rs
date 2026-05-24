use rsx_cast::WalHeader;
use rsx_cast::header::WAL_HEADER_VERSION_LATEST;
use rsx_cast::header::WAL_HEADER_VERSION_V1;

#[test]
fn header_encode_decode_roundtrip() {
    let header = WalHeader::new(1, 64, 0xDEADBEEF);
    let bytes = header.to_bytes();
    let decoded = WalHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.version, WAL_HEADER_VERSION_LATEST);
    assert_eq!(decoded.record_type, 1);
    assert_eq!(decoded.len, 64);
    assert_eq!(decoded.crc32, 0xDEADBEEF);
    assert_eq!(decoded._reserved, [0u8; 7]);
}

#[test]
fn header_little_endian_verified() {
    // Wire format: byte 0 = version, byte 1 = pad,
    // bytes 2..4 = record_type, bytes 4..6 = len,
    // bytes 6..8 = pad, bytes 8..12 = crc32, 12..16 = reserved.
    let raw: [u8; 16] = [
        WAL_HEADER_VERSION_V1, // version
        0x00,                  // pad
        0x02, 0x01,            // record_type = 0x0102 LE
        0x03, 0x04,            // len = 0x0403 LE
        0x00, 0x00,            // pad
        0x05, 0x06, 0x07, 0x08, // crc32 LE
        0x00, 0x00, 0x00, 0x00, // reserved
    ];
    let h = WalHeader::from_bytes(&raw).unwrap();
    assert_eq!(h.version, WAL_HEADER_VERSION_V1);
    assert_eq!(h.record_type, 0x0102);
    assert_eq!(h.len, 0x0403);
    assert_eq!(h.crc32, 0x08070605);
    assert_eq!(h._reserved, [0u8; 7]);
}

#[test]
fn header_new_writes_latest_version() {
    let header = WalHeader::new(1, 0, 0);
    assert_eq!(header.version, WAL_HEADER_VERSION_LATEST);
    assert_eq!(WAL_HEADER_VERSION_LATEST, WAL_HEADER_VERSION_V1);
}

#[test]
fn header_version_zero_rejected() {
    // version=0 at byte 0 is not a known version.
    let raw: [u8; 16] = [
        0x00, 0x00,
        0x01, 0x00,
        0x40, 0x00,
        0x00, 0x00,
        0xAA, 0xBB, 0xCC, 0xDD,
        0x00, 0x00, 0x00, 0x00,
    ];
    let h = WalHeader::from_bytes(&raw).unwrap();
    assert_eq!(h.version, 0);
    assert!(
        !h.is_supported_version(),
        "version 0 must be rejected"
    );
}

#[test]
fn header_unknown_version_rejected() {
    let raw: [u8; 16] = [
        0xFF, // unknown version
        0x00, // pad
        0x01, 0x00, // record_type = 1
        0x40, 0x00, // len = 64
        0x00, 0x00, // pad
        0xAA, 0xBB, 0xCC, 0xDD, // crc32
        0x00, 0x00, 0x00, 0x00, // reserved
    ];
    let h = WalHeader::from_bytes(&raw).unwrap();
    assert_eq!(h.version, 0xFF);
    assert!(
        !h.is_supported_version(),
        "unknown version must be rejected"
    );
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
    use rsx_cast::compute_crc32;
    let payload = b"test payload data";
    let crc = compute_crc32(payload);
    let header = WalHeader::new(1, payload.len() as u16, crc);
    assert_eq!(header.crc32, crc);
}

/// Stress the decoder with random byte sequences to catch
/// any panic, OOB access, or arithmetic overflow in
/// `from_bytes`. Deterministic seeded LCG.
#[test]
fn header_from_bytes_no_panic_on_random_input() {
    let mut state: u64 = 0xCAFEBABEu64;
    let mut buf = [0u8; 16];
    for _ in 0..100_000 {
        for b in buf.iter_mut() {
            // splitmix64
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30))
                .wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27))
                .wrapping_mul(0x94D049BB133111EB);
            *b = (z ^ (z >> 31)) as u8;
        }
        let _ = WalHeader::from_bytes(&buf);
    }
}

/// from_bytes must return None for any slice shorter than
/// 16, never panic.
#[test]
fn header_from_bytes_no_panic_on_short_input() {
    for len in 0..16 {
        let buf = vec![0xFFu8; len];
        assert!(WalHeader::from_bytes(&buf).is_none());
    }
}
