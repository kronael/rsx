//! The RSX exchange's application wire records — fixed
//! `#[repr(C, align(64))]` structs that flow over the `rsx-cast` transport with
//! no serialization step.
//!
//! Eleven record types (fills, BBO, the order lifecycle, marks, liquidations)
//! each implement [`rsx_cast::CastRecord`], so the same bytes ride casting/UDP,
//! replication/TCP, and the WAL unchanged — wire = stream = disk. Every record's
//! first field is `seq: u64` (the transport stamps it in place) and its size is
//! locked at compile time with `const _` assertions. RSX-specific: reusable only
//! alongside `rsx-cast` and `rsx-types`. See ARCHITECTURE.md and, in the wider
//! rsx repo, `specs/2/18-messages.md` for the per-field reference.

use rsx_cast::as_bytes;
use rsx_cast::encode_record;
use rsx_cast::CastRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::mem;

/// Record type constants — domain layer (transport-level
/// constants live in `rsx_cast::protocol`).
pub const RECORD_FILL: u16 = 0;
pub const RECORD_BBO: u16 = 1;
pub const RECORD_ORDER_INSERTED: u16 = 2;
pub const RECORD_ORDER_CANCELLED: u16 = 3;
pub const RECORD_ORDER_DONE: u16 = 4;
pub const RECORD_CONFIG_APPLIED: u16 = 5;
pub const RECORD_ORDER_ACCEPTED: u16 = 7;
pub const RECORD_MARK_PRICE: u16 = 8;
pub const RECORD_ORDER_REQUEST: u16 = 9;
pub const RECORD_ORDER_RESPONSE: u16 = 10;
pub const RECORD_CANCEL_REQUEST: u16 = 11;
pub const RECORD_ORDER_FAILED: u16 = 12;
pub const RECORD_LIQUIDATION: u16 = 13;

/// CancelReason enum (u8)
pub const CANCEL_REASON_USER_CANCEL: u8 = 0;
pub const CANCEL_REASON_REDUCE_ONLY: u8 = 1;
pub const CANCEL_REASON_EXPIRY: u8 = 2;
pub const CANCEL_REASON_SYSTEM: u8 = 3;
pub const CANCEL_REASON_POST_ONLY_REJECT: u8 = 4;
pub const CANCEL_REASON_OTHER: u8 = 5;

/// FillRecord (64-byte aligned).
///
/// Carries the order lifecycle's per-hop latency timestamps
/// (specs/2/59-latency-observability.md): each hop stamps only
/// its own field as the record travels `GW -> Risk -> ME ->
/// Risk -> GW`, so a consumer derives whatever delta it wants
/// (`engine = match_done_ns minus me_in_ns`, `internal =
/// gw_out_ns minus gw_in_ns`) without a bespoke sidecar frame.
/// A duration measured within one host is always valid; a
/// cross-hop subtraction assumes clock-synced hosts (true under
/// PTP in production, trivially true on a single-box dev
/// setup). An unset field is `0` — consumers must treat `0` as
/// "unset", not as a real timestamp.
///
/// `gw_in_ns` (generalised from the former `taker_ts_ns`) is
/// the taker order's gateway-ingress timestamp (echoed from
/// `OrderRequest.timestamp_ns`). Older WAL records may have
/// this slot as uninitialized memory; consumers must validate
/// (non-zero + plausible epoch) and fall back to `ts_ns` if
/// invalid.
///
/// `risk_in_ns` is always `0` today: the risk tile's ingress
/// timestamp cannot reach this record without growing the
/// risk->ME wire struct (`OrderMessage`, exactly 64 bytes, zero
/// spare capacity) — a separate, not-yet-authorized wire
/// change. See ARCHITECTURE.md / the 59-latency-observability
/// increment-2 notes.
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
    pub gw_in_ns: u64,
    /// Always 0 today — see struct doc.
    pub risk_in_ns: u64,
    pub me_in_ns: u64,
    pub match_done_ns: u64,
    pub gw_out_ns: u64,
}

impl CastRecord for FillRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_FILL
    }
}

const _: () = assert!(mem::size_of::<FillRecord>() == 128); // unchanged: new fields fill former tail padding
const _: () = assert!(mem::align_of::<FillRecord>() == 64);

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

impl CastRecord for BboRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_BBO
    }
}

const _: () = assert!(mem::size_of::<BboRecord>() == 128);
const _: () = assert!(mem::align_of::<BboRecord>() == 64);

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

impl CastRecord for OrderInsertedRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_INSERTED
    }
}

const _: () = assert!(mem::size_of::<OrderInsertedRecord>() == 64);
const _: () = assert!(mem::align_of::<OrderInsertedRecord>() == 64);

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

impl CastRecord for OrderCancelledRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_CANCELLED
    }
}

const _: () = assert!(mem::size_of::<OrderCancelledRecord>() == 64);
const _: () = assert!(mem::align_of::<OrderCancelledRecord>() == 64);

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

impl CastRecord for OrderDoneRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_DONE
    }
}

const _: () = assert!(mem::size_of::<OrderDoneRecord>() == 64);
const _: () = assert!(mem::align_of::<OrderDoneRecord>() == 64);

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

impl CastRecord for ConfigAppliedRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_CONFIG_APPLIED
    }
}

const _: () = assert!(mem::size_of::<ConfigAppliedRecord>() == 64);
const _: () = assert!(mem::align_of::<ConfigAppliedRecord>() == 64);

/// OrderAcceptedRecord (64-byte aligned)
/// Dedup key is (user_id, order_id). Contains full order
/// fields so WAL replay can reconstruct frozen margin
/// without Postgres.
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
    pub cid: [u8; 20],
}

impl CastRecord for OrderAcceptedRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_ACCEPTED
    }
}

const _: () = assert!(mem::size_of::<OrderAcceptedRecord>() == 128);
const _: () = assert!(mem::align_of::<OrderAcceptedRecord>() == 64);

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

impl CastRecord for MarkPriceRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_MARK_PRICE
    }
}

const _: () = assert!(mem::size_of::<MarkPriceRecord>() == 64);
const _: () = assert!(mem::align_of::<MarkPriceRecord>() == 64);

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

impl CastRecord for OrderFailedRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_FAILED
    }
}

const _: () = assert!(mem::size_of::<OrderFailedRecord>() == 64);
const _: () = assert!(mem::align_of::<OrderFailedRecord>() == 64);

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

impl CastRecord for CancelRequest {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_CANCEL_REQUEST
    }
}

const _: () = assert!(mem::size_of::<CancelRequest>() == 64);
const _: () = assert!(mem::align_of::<CancelRequest>() == 64);

/// Inbound order from the risk engine to a matching engine — the
/// `RECORD_ORDER_REQUEST` payload on the risk→ME hop (risk builds it, ME
/// decodes it, sent via `send_raw`). Per-symbol, so no `symbol_id` and no
/// liquidation flag; risk resolved those upstream.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrderMessage {
    pub seq: u64,
    pub price: i64,
    pub qty: i64,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
    pub user_id: u32,
    pub _pad2: u32,
    pub timestamp_ns: u64,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
}

const _: () = assert!(mem::size_of::<OrderMessage>() == 64);

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

impl CastRecord for LiquidationRecord {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_LIQUIDATION
    }
}

const _: () = assert!(mem::size_of::<LiquidationRecord>() == 64);
const _: () = assert!(mem::align_of::<LiquidationRecord>() == 64);

// Per-type encode helpers. Wrap the generic
// `rsx_cast::encode_record` with the matching record_type.

pub fn encode_fill_record(record: &FillRecord) -> Vec<u8> {
    encode_record(RECORD_FILL, as_bytes(record))
}

pub fn encode_bbo_record(record: &BboRecord) -> Vec<u8> {
    encode_record(RECORD_BBO, as_bytes(record))
}

pub fn encode_order_inserted_record(record: &OrderInsertedRecord) -> Vec<u8> {
    encode_record(RECORD_ORDER_INSERTED, as_bytes(record))
}

pub fn encode_order_cancelled_record(record: &OrderCancelledRecord) -> Vec<u8> {
    encode_record(RECORD_ORDER_CANCELLED, as_bytes(record))
}

pub fn encode_order_done_record(record: &OrderDoneRecord) -> Vec<u8> {
    encode_record(RECORD_ORDER_DONE, as_bytes(record))
}

pub fn encode_config_applied_record(record: &ConfigAppliedRecord) -> Vec<u8> {
    encode_record(RECORD_CONFIG_APPLIED, as_bytes(record))
}

pub fn encode_order_accepted_record(record: &OrderAcceptedRecord) -> Vec<u8> {
    encode_record(RECORD_ORDER_ACCEPTED, as_bytes(record))
}

pub fn encode_order_failed_record(record: &OrderFailedRecord) -> Vec<u8> {
    encode_record(RECORD_ORDER_FAILED, as_bytes(record))
}

macro_rules! decode_record {
    ($name:ident, $ty:ty) => {
        pub fn $name(payload: &[u8]) -> Option<$ty> {
            if payload.len() < mem::size_of::<$ty>() {
                return None;
            }
            Some(unsafe { std::ptr::read_unaligned(payload.as_ptr() as *const $ty) })
        }
    };
}

decode_record!(decode_fill_record, FillRecord);
decode_record!(decode_bbo_record, BboRecord);
decode_record!(decode_order_inserted_record, OrderInsertedRecord);
decode_record!(decode_order_cancelled_record, OrderCancelledRecord);
decode_record!(decode_order_done_record, OrderDoneRecord);
decode_record!(decode_config_applied_record, ConfigAppliedRecord);
decode_record!(decode_order_failed_record, OrderFailedRecord);
decode_record!(decode_order_accepted_record, OrderAcceptedRecord);
