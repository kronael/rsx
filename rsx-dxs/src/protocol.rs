//! CMP/DXS protocol records.
//!
//! Domain wire records (FillRecord, BboRecord, …) live in
//! the `rsx-messages` crate. This module holds only the
//! protocol-level records the transport itself emits and
//! consumes.

use std::mem;

/// Transport-level record type constants.
///
/// NOTE: `0x10` was previously `RECORD_STATUS_MESSAGE`
/// (receiver → sender flow control). It was removed when
/// backpressure was deemed an anti-pattern for exchange-grade
/// NAK+UDP transports. Do NOT reuse `0x10` — keep it
/// reserved so older receivers that decode it silently
/// don't get confused.
pub const RECORD_CAUGHT_UP: u16 = 6;
pub const RECORD_NAK: u16 = 0x11;
pub const RECORD_HEARTBEAT: u16 = 0x12;
pub const RECORD_REPLAY_REQUEST: u16 = 0x13;

/// Trait for all CMP data records. Guarantees seq is
/// readable/writable at a known location in the payload.
/// All implementors are #[repr(C, align(64))], Copy.
pub trait CmpRecord: Copy {
    fn seq(&self) -> u64;
    fn set_seq(&mut self, seq: u64);
    fn record_type() -> u16;
}

/// CmpHeartbeat (64-byte aligned)
/// Sender -> receiver, every 10ms.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CmpHeartbeat {
    pub highest_seq: u64,
    pub _pad1: [u8; 56],
}
const _: () = assert!(mem::size_of::<CmpHeartbeat>() == 64);
const _: () = assert!(mem::align_of::<CmpHeartbeat>() == 64);

/// CMP Nak (64-byte aligned)
/// Receiver -> sender, on gap detection.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Nak {
    pub from_seq: u64,
    pub count: u64,
    pub _pad1: [u8; 48],
}
const _: () = assert!(mem::size_of::<Nak>() == 64);
const _: () = assert!(mem::align_of::<Nak>() == 64);

/// ReplayRequest (64-byte aligned)
/// Client -> server for WAL/TCP replay. Keeps stream_id
/// (TCP routing).
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ReplayRequest {
    pub stream_id: u32,
    pub _pad0: u32,
    pub from_seq: u64,
    pub _pad1: [u8; 48],
}
const _: () = assert!(mem::size_of::<ReplayRequest>() == 64);
const _: () = assert!(mem::align_of::<ReplayRequest>() == 64);

/// CaughtUpRecord (64-byte aligned)
/// TCP replay control: server emits this to mark the
/// transition from historical replay to live tail.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CaughtUpRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub stream_id: u32,
    pub _pad0: u32,
    pub live_seq: u64,
    pub _pad1: [u8; 40],
}
const _: () = assert!(mem::size_of::<CaughtUpRecord>() == 128);
const _: () = assert!(mem::align_of::<CaughtUpRecord>() == 64);

impl CmpRecord for CaughtUpRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_CAUGHT_UP }
}
