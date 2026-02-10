use uuid::Uuid;

/// Generate a UUIDv7 order ID as 16-byte array.
/// Monotonic within millisecond, time-sortable.
pub fn generate_order_id() -> [u8; 16] {
    let id = Uuid::now_v7();
    *id.as_bytes()
}

/// Format order ID as 32-char lowercase hex string.
pub fn order_id_to_hex(id: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for byte in id {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", byte);
    }
    s
}

/// Parse 32-char hex string to [u8; 16].
/// Returns None if invalid.
pub fn hex_to_order_id(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut id = [0u8; 16];
    for i in 0..16 {
        id[i] = u8::from_str_radix(
            &hex[i * 2..i * 2 + 2],
            16,
        )
        .ok()?;
    }
    Some(id)
}

/// Extract millisecond timestamp from UUIDv7.
pub fn order_id_timestamp_ms(id: &[u8; 16]) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[2..8].copy_from_slice(&id[0..6]);
    u64::from_be_bytes(bytes)
}
