use uuid::Uuid;

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Generate a UUIDv7 order ID as 16-byte array.
/// Monotonic within millisecond, time-sortable.
pub fn generate_order_id() -> [u8; 16] {
    let id = Uuid::now_v7();
    *id.as_bytes()
}

/// Split an order ID into the big-endian `(hi, lo)` halves the wire
/// records carry.
pub fn split_order_id(id: &[u8; 16]) -> (u64, u64) {
    let value = u128::from_be_bytes(*id);
    ((value >> 64) as u64, value as u64)
}

/// Join the `(hi, lo)` halves of a wire record into an order ID.
pub fn join_order_id(hi: u64, lo: u64) -> [u8; 16] {
    ((u128::from(hi) << 64) | u128::from(lo)).to_be_bytes()
}

/// Format order ID as 32-char lowercase hex string.
pub fn order_id_to_hex(id: &[u8; 16]) -> String {
    id.iter()
        .flat_map(|byte| {
            [
                char::from(HEX_DIGITS[usize::from(byte >> 4)]),
                char::from(HEX_DIGITS[usize::from(byte & 0x0F)]),
            ]
        })
        .collect()
}

/// Parse 32-char hex string to [u8; 16].
/// Returns None if invalid.
pub fn hex_to_order_id(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut id = [0u8; 16];
    for i in 0..16 {
        id[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(id)
}

/// Extract millisecond timestamp from UUIDv7.
pub fn order_id_timestamp_ms(id: &[u8; 16]) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[2..8].copy_from_slice(&id[0..6]);
    u64::from_be_bytes(bytes)
}
