/// Record type constants
pub const RECORD_FILL: u16 = 0;
pub const RECORD_BBO: u16 = 1;
pub const RECORD_ORDER_INSERTED: u16 = 2;
pub const RECORD_ORDER_CANCELLED: u16 = 3;
pub const RECORD_ORDER_DONE: u16 = 4;
pub const RECORD_CONFIG_APPLIED: u16 = 5;
pub const RECORD_CAUGHT_UP: u16 = 6;
pub const RECORD_ORDER_ACCEPTED: u16 = 7;
pub const RECORD_MARK_PRICE: u16 = 8;
pub const RECORD_STATUS_MESSAGE: u16 = 0x10;
pub const RECORD_NAK: u16 = 0x11;
pub const RECORD_HEARTBEAT: u16 = 0x12;
pub const RECORD_REPLAY_REQUEST: u16 = 0x13;

/// Common prefix for all CMP data payloads.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PayloadPreamble {
    pub seq: u64,
    pub ver: u16,
    pub kind: u8,
    pub _pad0: u8,
    pub len: u32,
}

impl PayloadPreamble {
    pub const SIZE: usize = 16;
}

/// CancelReason enum (u8)
pub const CANCEL_REASON_USER_CANCEL: u8 = 0;
pub const CANCEL_REASON_REDUCE_ONLY: u8 = 1;
pub const CANCEL_REASON_EXPIRY: u8 = 2;
pub const CANCEL_REASON_SYSTEM: u8 = 3;
pub const CANCEL_REASON_POST_ONLY_REJECT: u8 = 4;
pub const CANCEL_REASON_OTHER: u8 = 5;

/// FillRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct FillRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub _pad0: u32,
    pub taker_order_id_hi: u64,
    pub taker_order_id_lo: u64,
    pub maker_order_id_hi: u64,
    pub maker_order_id_lo: u64,
    pub price: i64,
    pub qty: i64,
    pub taker_side: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

/// BboRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct BboRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub _pad0: u32,
    pub bid_px: i64,
    pub bid_qty: i64,
    pub bid_count: u32,
    pub _pad1: u32,
    pub ask_px: i64,
    pub ask_qty: i64,
    pub ask_count: u32,
    pub _pad2: u32,
}

/// OrderInsertedRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderInsertedRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub price: i64,
    pub qty: i64,
    pub side: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

/// OrderCancelledRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderCancelledRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub remaining_qty: i64,
    pub reason: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

/// OrderDoneRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderDoneRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub filled_qty: i64,
    pub remaining_qty: i64,
    pub final_status: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

/// ConfigAppliedRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ConfigAppliedRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub _pad0: u32,
    pub config_version: u64,
    pub effective_at_ms: u64,
    pub applied_at_ns: u64,
}

/// CaughtUpRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CaughtUpRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub stream_id: u32,
    pub _pad0: u32,
    pub live_seq: u64,
    pub _pad1: [u8; 40],
}

/// OrderAcceptedRecord (64 bytes aligned)
/// Spec: DXS.md section 1 payload layouts
/// Dedup key is (user_id, order_id)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderAcceptedRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub user_id: u32,
    pub _pad0: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub _pad1: [u8; 32],
}

/// MarkPriceRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct MarkPriceRecord {
    pub preamble: PayloadPreamble,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub _pad0: u32,
    pub mark_price: i64,
    pub index_price: i64,
    pub _pad1: [u8; 24],
}

/// CMP StatusMessage (64 bytes aligned)
/// Receiver -> sender, every 10ms
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct StatusMessage {
    pub stream_id: u32,
    pub _pad0: u32,
    pub consumption_seq: u64,
    pub receiver_window: u64,
    pub _pad1: [u8; 40],
}

/// CMP Nak (64 bytes aligned)
/// Receiver -> sender, on gap detection
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Nak {
    pub stream_id: u32,
    pub _pad0: u32,
    pub from_seq: u64,
    pub count: u64,
    pub _pad1: [u8; 40],
}

/// CMP Heartbeat (64 bytes aligned)
/// Sender -> receiver, every 10ms
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CmpHeartbeat {
    pub stream_id: u32,
    pub _pad0: u32,
    pub highest_seq: u64,
    pub _pad1: [u8; 48],
}

/// ReplayRequest (64 bytes aligned)
/// Client -> server for WAL/TCP replay
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ReplayRequest {
    pub stream_id: u32,
    pub _pad0: u32,
    pub from_seq: u64,
    pub _pad1: [u8; 48],
}

/// Enum for all WAL record types
#[derive(Debug, Clone)]
pub enum WalRecord {
    Fill(FillRecord),
    Bbo(BboRecord),
    OrderInserted(OrderInsertedRecord),
    OrderCancelled(OrderCancelledRecord),
    OrderDone(OrderDoneRecord),
    ConfigApplied(ConfigAppliedRecord),
    CaughtUp(CaughtUpRecord),
    OrderAccepted(OrderAcceptedRecord),
}

impl WalRecord {
    pub fn seq(&self) -> u64 {
        match self {
            WalRecord::Fill(r) => r.preamble.seq,
            WalRecord::Bbo(r) => r.preamble.seq,
            WalRecord::OrderInserted(r) => r.preamble.seq,
            WalRecord::OrderCancelled(r) => r.preamble.seq,
            WalRecord::OrderDone(r) => r.preamble.seq,
            WalRecord::ConfigApplied(r) => r.preamble.seq,
            WalRecord::CaughtUp(r) => r.preamble.seq,
            WalRecord::OrderAccepted(r) => r.preamble.seq,
        }
    }

    pub fn record_type(&self) -> u16 {
        match self {
            WalRecord::Fill(_) => RECORD_FILL,
            WalRecord::Bbo(_) => RECORD_BBO,
            WalRecord::OrderInserted(_) => RECORD_ORDER_INSERTED,
            WalRecord::OrderCancelled(_) => RECORD_ORDER_CANCELLED,
            WalRecord::OrderDone(_) => RECORD_ORDER_DONE,
            WalRecord::ConfigApplied(_) => RECORD_CONFIG_APPLIED,
            WalRecord::CaughtUp(_) => RECORD_CAUGHT_UP,
            WalRecord::OrderAccepted(_) => RECORD_ORDER_ACCEPTED,
        }
    }
}
