//! CMP/DXS protocol records.
//!
//! Domain wire records (FillRecord, BboRecord, …) live in
//! the `rsx-messages` crate. This module holds only the
//! protocol-level records the transport itself emits and
//! consumes.
//!
//! ## Wire-format discipline
//!
//! Every record is `#[repr(C, align(64))]` with explicit
//! `_pad…` fields chosen so that `mem::size_of::<T>()` lands
//! on the documented wire size (64 or 128 bytes). If you
//! change a struct, **adjust the padding so the size stays
//! exactly what the wire expects** — receivers on the other
//! end parse a fixed number of bytes; a silent size drift
//! breaks the protocol without breaking the build.
//!
//! The `align(64)` is compiler-enforced; size is not. Read
//! the size annotation next to each struct before adding or
//! changing fields.

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

/// CmpHeartbeat — wire size 64 bytes.
/// Sender -> receiver, when stream is idle.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CmpHeartbeat {
    pub highest_seq: u64,
    pub _pad1: [u8; 56],
}

/// CMP Nak — wire size 64 bytes.
/// Receiver -> sender, on gap detection.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Nak {
    pub from_seq: u64,
    pub count: u64,
    pub _pad1: [u8; 48],
}

/// ReplayRequest — wire size 64 bytes.
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

/// CaughtUpRecord — wire size 128 bytes.
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

impl CmpRecord for CaughtUpRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_CAUGHT_UP }
}
