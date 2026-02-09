/// Record type constants
pub const RECORD_FILL: u16 = 0;
pub const RECORD_BBO: u16 = 1;
pub const RECORD_ORDER_INSERTED: u16 = 2;
pub const RECORD_ORDER_CANCELLED: u16 = 3;
pub const RECORD_ORDER_DONE: u16 = 4;
pub const RECORD_CONFIG_APPLIED: u16 = 5;
pub const RECORD_CAUGHT_UP: u16 = 6;
pub const RECORD_ORDER_ACCEPTED: u16 = 7;

/// FillRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct FillRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub maker_oid: u128,
    pub taker_oid: u128,
    pub px: i64,
    pub qty: i64,
    pub maker_side: u8,
    pub _pad1: [u8; 7],
}

/// BboRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct BboRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub bid_px: i64,
    pub ask_px: i64,
    pub bid_qty: i64,
    pub ask_qty: i64,
    pub _pad1: [u8; 4],
}

/// OrderInsertedRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderInsertedRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub oid: u128,
    pub user_id: u32,
    pub px: i64,
    pub qty: i64,
    pub side: u8,
    pub _pad1: [u8; 7],
}

/// OrderCancelledRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderCancelledRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub oid: u128,
    pub reason: u8,
    pub _pad1: [u8; 7],
}

/// OrderDoneRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderDoneRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub oid: u128,
    pub remaining_qty: i64,
    pub reason: u8,
    pub _pad1: [u8; 7],
}

/// ConfigAppliedRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ConfigAppliedRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub config_version: u32,
    pub _pad1: [u8; 40],
}

/// CaughtUpRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CaughtUpRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub stream_id: u32,
    pub live_seq: u64,
    pub _pad1: [u8; 36],
}

/// OrderAcceptedRecord (64 bytes aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderAcceptedRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub oid: u128,
    pub cid: [u8; 20],
    pub user_id: u32,
    pub _pad1: [u8; 20],
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
