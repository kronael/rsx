use crate::header::WalHeader;
use crc32fast::Hasher;
use std::mem;

pub fn compute_crc32(payload: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(payload);
    hasher.finalize()
}

pub fn encode_record(
    record_type: u16,
    payload: &[u8],
) -> Vec<u8> {
    let crc32 = compute_crc32(payload);
    let header = WalHeader::new(
        record_type,
        payload.len() as u16,
        crc32,
    );

    let mut buf = Vec::with_capacity(
        WalHeader::SIZE + payload.len(),
    );
    buf.extend_from_slice(&header.to_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Convert repr(C) struct to byte slice. Used for
/// serializing CMP records to wire format.
pub fn as_bytes<T>(val: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            val as *const T as *const u8,
            mem::size_of::<T>(),
        )
    }
}

/// Generic decode helper. Domain crates should wrap this
/// with their own typed `decode_*` helpers (see
/// `rsx-messages` for the RSX exchange records).
pub fn decode_payload<T: Copy>(payload: &[u8]) -> Option<T> {
    if payload.len() < mem::size_of::<T>() {
        return None;
    }
    Some(unsafe {
        std::ptr::read_unaligned(payload.as_ptr() as *const T)
    })
}
