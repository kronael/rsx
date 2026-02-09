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
    pub seq: u64,
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
    pub seq: u64,
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
    pub seq: u64,
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
    pub seq: u64,
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
    pub seq: u64,
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
    pub seq: u64,
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
    pub seq: u64,
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
    pub seq: u64,
    pub ts_ns: u64,
    pub user_id: u32,
    pub _pad0: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub _pad1: [u8; 32],
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
            WalRecord::Fill(r) => r.seq,
            WalRecord::Bbo(r) => r.seq,
            WalRecord::OrderInserted(r) => r.seq,
            WalRecord::OrderCancelled(r) => r.seq,
            WalRecord::OrderDone(r) => r.seq,
            WalRecord::ConfigApplied(r) => r.seq,
            WalRecord::CaughtUp(r) => r.seq,
            WalRecord::OrderAccepted(r) => r.seq,
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
