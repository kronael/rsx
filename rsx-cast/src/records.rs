//! Transport protocol records + `CastRecord` trait. See `specs/4-cast.md`.
//!
//! Wire-format discipline: every record is `#[repr(C, align(64))]` with explicit
//! `_pad…` fields so `mem::size_of::<T>()` lands on the documented wire size.
//! `align(64)` is compiler-enforced; size is not. Adjust padding when fields change
//! — receivers parse a fixed number of bytes and size drift silently breaks the wire.

/// Transport-level record type constants.
///
/// `0x10` is a reserved gap; do not assign a record type to it.
pub const RECORD_CAUGHT_UP: u16 = 6;
pub const RECORD_NAK: u16 = 0x11;
pub const RECORD_HEARTBEAT: u16 = 0x12;
pub(crate) const RECORD_REPLICATION_REQUEST: u16 = 0x13;
pub(crate) const RECORD_REPLICATION_NOT_AVAILABLE: u16 = 0x15;

/// Trait for all casting data records. Guarantees seq is
/// readable/writable at a known location in the payload.
/// All implementors are #[repr(C, align(64))], Copy.
pub trait CastRecord: Copy {
    fn seq(&self) -> u64;
    fn set_seq(&mut self, seq: u64);
    fn record_type() -> u16;
}

/// CastHeartbeat — wire size 64 bytes.
/// Sender -> receiver, when stream is idle.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CastHeartbeat {
    pub highest_seq: u64,
    pub _pad1: [u8; 56],
}

/// Cast Nak — wire size 64 bytes.
/// Receiver -> sender, on gap detection.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Nak {
    pub from_seq: u64,
    pub count: u64,
    pub _pad1: [u8; 48],
}

/// ReplicationRequest — wire size 64 bytes.
/// Client -> server for WAL/TCP replay. Keeps stream_id
/// (TCP routing).
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplicationRequest {
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

impl CastRecord for CaughtUpRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_CAUGHT_UP }
}

/// ReplicationNotAvailable — wire size 64 bytes.
///
/// Server -> client when the requested `from_seq` is outside
/// the range this endpoint can serve (below the oldest seq on
/// disk for this stream). The server emits this and closes
/// the connection cleanly; the consumer should try the next
/// endpoint in its list with the same `from_seq`.
///
/// Federation: producer + recorder-archive(s) form a tiered
/// chain — producer holds the live tail, archive(s) hold cold
/// history. A consumer with the right endpoint list can fall
/// back from producer (which may have GC'd the requested seq)
/// to an archive that still holds it.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplicationNotAvailable {
    pub requested_from_seq: u64,
    /// Floor this endpoint can serve. 0 = endpoint is empty
    /// (no records on disk for this stream).
    pub my_oldest_seq: u64,
    /// Ceiling currently on disk (last_seq of the newest
    /// rotated/active file). 0 = endpoint is empty.
    pub my_highest_seq: u64,
    pub stream_id: u32,
    pub _pad: [u8; 36],
}
