use crate::header::WalHeader;
use crate::records::*;
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

macro_rules! decode_record {
    ($name:ident, $ty:ty) => {
        pub fn $name(payload: &[u8]) -> Option<$ty> {
            if payload.len() < mem::size_of::<$ty>() {
                return None;
            }
            Some(unsafe {
                std::ptr::read_unaligned(
                    payload.as_ptr() as *const $ty,
                )
            })
        }
    };
}

pub fn encode_fill_record(
    record: &FillRecord,
) -> Vec<u8> {
    encode_record(RECORD_FILL, as_bytes(record))
}

pub fn encode_bbo_record(
    record: &BboRecord,
) -> Vec<u8> {
    encode_record(RECORD_BBO, as_bytes(record))
}

pub fn encode_order_inserted_record(
    record: &OrderInsertedRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_ORDER_INSERTED, as_bytes(record),
    )
}

pub fn encode_order_cancelled_record(
    record: &OrderCancelledRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_ORDER_CANCELLED, as_bytes(record),
    )
}

pub fn encode_order_done_record(
    record: &OrderDoneRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_ORDER_DONE, as_bytes(record),
    )
}

pub fn encode_config_applied_record(
    record: &ConfigAppliedRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_CONFIG_APPLIED, as_bytes(record),
    )
}

pub fn encode_caught_up_record(
    record: &CaughtUpRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_CAUGHT_UP, as_bytes(record),
    )
}

pub fn encode_order_accepted_record(
    record: &OrderAcceptedRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_ORDER_ACCEPTED, as_bytes(record),
    )
}

pub fn encode_order_failed_record(
    record: &OrderFailedRecord,
) -> Vec<u8> {
    encode_record(
        RECORD_ORDER_FAILED, as_bytes(record),
    )
}

decode_record!(decode_fill_record, FillRecord);
decode_record!(decode_bbo_record, BboRecord);
decode_record!(
    decode_order_inserted_record,
    OrderInsertedRecord
);
decode_record!(
    decode_order_cancelled_record,
    OrderCancelledRecord
);
decode_record!(
    decode_order_done_record,
    OrderDoneRecord
);
decode_record!(
    decode_config_applied_record,
    ConfigAppliedRecord
);
decode_record!(
    decode_caught_up_record,
    CaughtUpRecord
);
decode_record!(
    decode_order_failed_record,
    OrderFailedRecord
);
decode_record!(
    decode_order_accepted_record,
    OrderAcceptedRecord
);
