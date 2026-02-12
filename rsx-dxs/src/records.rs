use rsx_types::Price;
use rsx_types::Qty;

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
pub const RECORD_ORDER_REQUEST: u16 = 9;
pub const RECORD_ORDER_RESPONSE: u16 = 10;
pub const RECORD_CANCEL_REQUEST: u16 = 11;
pub const RECORD_ORDER_FAILED: u16 = 12;
pub const RECORD_LIQUIDATION: u16 = 13;
pub const RECORD_STATUS_MESSAGE: u16 = 0x10;
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

/// CancelReason enum (u8)
pub const CANCEL_REASON_USER_CANCEL: u8 = 0;
pub const CANCEL_REASON_REDUCE_ONLY: u8 = 1;
pub const CANCEL_REASON_EXPIRY: u8 = 2;
pub const CANCEL_REASON_SYSTEM: u8 = 3;
pub const CANCEL_REASON_POST_ONLY_REJECT: u8 = 4;
pub const CANCEL_REASON_OTHER: u8 = 5;

/// FillRecord (64-byte aligned)
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
    pub price: Price,
    pub qty: Qty,
    pub taker_side: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

impl CmpRecord for FillRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_FILL }
}

/// BboRecord (64-byte aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct BboRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub _pad0: u32,
    pub bid_px: Price,
    pub bid_qty: Qty,
    pub bid_count: u32,
    pub _pad1: u32,
    pub ask_px: Price,
    pub ask_qty: Qty,
    pub ask_count: u32,
    pub _pad2: u32,
}

impl CmpRecord for BboRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_BBO }
}

/// OrderInsertedRecord (64-byte aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderInsertedRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub price: Price,
    pub qty: Qty,
    pub side: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

impl CmpRecord for OrderInsertedRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_ORDER_INSERTED }
}

/// OrderCancelledRecord (64-byte aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderCancelledRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub remaining_qty: Qty,
    pub reason: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

impl CmpRecord for OrderCancelledRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_ORDER_CANCELLED }
}

/// OrderDoneRecord (64-byte aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderDoneRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub filled_qty: Qty,
    pub remaining_qty: Qty,
    pub final_status: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}

impl CmpRecord for OrderDoneRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_ORDER_DONE }
}

/// ConfigAppliedRecord (64-byte aligned)
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

impl CmpRecord for ConfigAppliedRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_CONFIG_APPLIED }
}

/// CaughtUpRecord (64-byte aligned)
/// Keeps stream_id (TCP coordination msg).
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

/// OrderAcceptedRecord (64-byte aligned)
/// Dedup key is (user_id, order_id).
/// Contains full order fields so WAL replay can
/// reconstruct frozen margin without Postgres.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderAcceptedRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub user_id: u32,
    pub symbol_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub price: i64,
    pub qty: i64,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: u8,
    pub post_only: u8,
    pub _pad1: [u8; 12],
}

impl CmpRecord for OrderAcceptedRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_ORDER_ACCEPTED }
}

/// MarkPriceRecord (64-byte aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct MarkPriceRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub _pad0: u32,
    pub mark_price: Price,
    pub source_mask: u32,
    pub source_count: u32,
    pub _pad1: [u8; 24],
}

impl CmpRecord for MarkPriceRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_MARK_PRICE }
}

/// OrderFailedRecord (64-byte aligned)
/// Sent by Risk when pre-trade check rejects an order.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderFailedRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub user_id: u32,
    pub _pad0: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub reason: u8,
    pub _pad: [u8; 23],
}

impl CmpRecord for OrderFailedRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_FAILED
    }
}

/// CancelRequest (64-byte aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CancelRequest {
    pub seq: u64,
    pub ts_ns: u64,
    pub user_id: u32,
    pub symbol_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub _pad: [u8; 24],
}

impl CmpRecord for CancelRequest {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) { self.seq = seq; }
    fn record_type() -> u16 { RECORD_CANCEL_REQUEST }
}

/// Liquidation notification from risk to gateway.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct LiquidationRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub user_id: u32,
    pub symbol_id: u32,
    pub status: u8,
    pub side: u8,
    pub _pad0: [u8; 2],
    pub round: u32,
    pub qty: i64,
    pub price: i64,
    pub slip_bps: i64,
}

impl CmpRecord for LiquidationRecord {
    fn seq(&self) -> u64 { self.seq }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_LIQUIDATION
    }
}

/// CMP StatusMessage (64-byte aligned)
/// Receiver -> sender, every 10ms
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct StatusMessage {
    pub consumption_seq: u64,
    pub receiver_window: u64,
    pub _pad1: [u8; 48],
}

/// CMP Nak (64-byte aligned)
/// Receiver -> sender, on gap detection
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Nak {
    pub from_seq: u64,
    pub count: u64,
    pub _pad1: [u8; 48],
}

/// CMP Heartbeat (64-byte aligned)
/// Sender -> receiver, every 10ms
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CmpHeartbeat {
    pub highest_seq: u64,
    pub _pad1: [u8; 56],
}

/// ReplayRequest (64-byte aligned)
/// Client -> server for WAL/TCP replay.
/// Keeps stream_id (TCP routing).
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
    OrderFailed(OrderFailedRecord),
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
            WalRecord::OrderFailed(r) => r.seq,
        }
    }

    pub fn record_type(&self) -> u16 {
        match self {
            WalRecord::Fill(_) => RECORD_FILL,
            WalRecord::Bbo(_) => RECORD_BBO,
            WalRecord::OrderInserted(_) => {
                RECORD_ORDER_INSERTED
            }
            WalRecord::OrderCancelled(_) => {
                RECORD_ORDER_CANCELLED
            }
            WalRecord::OrderDone(_) => RECORD_ORDER_DONE,
            WalRecord::ConfigApplied(_) => {
                RECORD_CONFIG_APPLIED
            }
            WalRecord::CaughtUp(_) => RECORD_CAUGHT_UP,
            WalRecord::OrderAccepted(_) => {
                RECORD_ORDER_ACCEPTED
            }
            WalRecord::OrderFailed(_) => {
                RECORD_ORDER_FAILED
            }
        }
    }
}
