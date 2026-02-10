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

pub fn decode_fill_record(
    payload: &[u8],
) -> Option<FillRecord> {
    if payload.len() < mem::size_of::<FillRecord>() {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr() as *const FillRecord,
        )
    };
    Some(record)
}

pub fn decode_bbo_record(
    payload: &[u8],
) -> Option<BboRecord> {
    if payload.len() < mem::size_of::<BboRecord>() {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr() as *const BboRecord,
        )
    };
    Some(record)
}

pub fn decode_order_inserted_record(
    payload: &[u8],
) -> Option<OrderInsertedRecord> {
    if payload.len()
        < mem::size_of::<OrderInsertedRecord>()
    {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr()
                as *const OrderInsertedRecord,
        )
    };
    Some(record)
}

pub fn decode_order_cancelled_record(
    payload: &[u8],
) -> Option<OrderCancelledRecord> {
    if payload.len()
        < mem::size_of::<OrderCancelledRecord>()
    {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr()
                as *const OrderCancelledRecord,
        )
    };
    Some(record)
}

pub fn decode_order_done_record(
    payload: &[u8],
) -> Option<OrderDoneRecord> {
    if payload.len()
        < mem::size_of::<OrderDoneRecord>()
    {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr()
                as *const OrderDoneRecord,
        )
    };
    Some(record)
}

pub fn decode_config_applied_record(
    payload: &[u8],
) -> Option<ConfigAppliedRecord> {
    if payload.len()
        < mem::size_of::<ConfigAppliedRecord>()
    {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr()
                as *const ConfigAppliedRecord,
        )
    };
    Some(record)
}

pub fn decode_caught_up_record(
    payload: &[u8],
) -> Option<CaughtUpRecord> {
    if payload.len()
        < mem::size_of::<CaughtUpRecord>()
    {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr()
                as *const CaughtUpRecord,
        )
    };
    Some(record)
}

pub fn decode_order_accepted_record(
    payload: &[u8],
) -> Option<OrderAcceptedRecord> {
    if payload.len()
        < mem::size_of::<OrderAcceptedRecord>()
    {
        return None;
    }
    let record = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr()
                as *const OrderAcceptedRecord,
        )
    };
    Some(record)
}
