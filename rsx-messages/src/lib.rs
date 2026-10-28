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

mod frames;
mod jwt;

pub use frames::parse;
pub use frames::serialize;
pub use frames::CancelKey;
pub use frames::ParseError;
pub use frames::WsFrame;
pub use jwt::mint_jwt;
pub use jwt::Claims;
pub use jwt::JWT_AUDIENCE;
pub use jwt::JWT_ISSUER;

use rsx_cast::as_bytes;
use rsx_cast::encode_record;
use rsx_cast::CastRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::mem;

macro_rules! assert_no_tail_padding {
    ($record:ty, $last:ident, $last_ty:ty) => {
        const _: () = assert!(
            mem::offset_of!($record, $last) + mem::size_of::<$last_ty>()
                == mem::size_of::<$record>()
        );
    };
}

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

/// Stream id of risk's gateway egress stream: the WAL directory under
/// risk's WAL dir and the id the gateway asks risk's replication
/// service for. Distinct from the stream id 0 that the cast senders
/// of risk and the gateway use for their order streams, so a NAK on
/// those never reads this WAL.
pub const GW_STREAM_ID: u32 = 200;

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
/// `risk_in_ns` is stamped when risk receives the order and is
/// carried through the matching engine unchanged.
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
assert_no_tail_padding!(FillRecord, gw_out_ns, u64);

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
    pub _pad3: [u8; 56],
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
assert_no_tail_padding!(BboRecord, _pad3, [u8; 56]);

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
assert_no_tail_padding!(OrderInsertedRecord, _pad1, [u8; 4]);

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
    pub _pad2: [u8; 8],
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
assert_no_tail_padding!(OrderCancelledRecord, _pad2, [u8; 8]);

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
    pub cancel_reason: u8,
    pub _pad1: [u8; 3],
    pub gw_in_ns: u64,
    pub risk_in_ns: u64,
    pub me_in_ns: u64,
    pub match_done_ns: u64,
    pub gw_out_ns: u64,
    pub _pad2: [u8; 24],
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

const _: () = assert!(mem::size_of::<OrderDoneRecord>() == 128);
const _: () = assert!(mem::align_of::<OrderDoneRecord>() == 64);
assert_no_tail_padding!(OrderDoneRecord, _pad2, [u8; 24]);

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
    pub _pad1: [u8; 16],
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
assert_no_tail_padding!(ConfigAppliedRecord, _pad1, [u8; 16]);

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
    pub _pad: [u8; 48],
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
assert_no_tail_padding!(OrderAcceptedRecord, _pad, [u8; 48]);

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
assert_no_tail_padding!(MarkPriceRecord, _pad1, [u8; 24]);

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
assert_no_tail_padding!(OrderFailedRecord, _pad, [u8; 23]);

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
assert_no_tail_padding!(CancelRequest, _pad, [u8; 24]);

/// Inbound order from the gateway to risk — the `RECORD_ORDER_REQUEST`
/// payload on the gateway→risk hop (the gateway builds it, risk decodes it,
/// sent via `send_raw`). `timestamp_ns` is the gateway-ingress stamp.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct OrderRequest {
    pub seq: u64,
    pub user_id: u32,
    pub symbol_id: u32,
    pub price: i64,
    pub qty: i64,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub timestamp_ns: u64,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: bool,
    pub post_only: bool,
    pub is_liquidation: bool,
    pub _pad: [u8; 3],
}

const _: () = assert!(mem::size_of::<OrderRequest>() == 64);
const _: () = assert!(mem::align_of::<OrderRequest>() == 64);
assert_no_tail_padding!(OrderRequest, _pad, [u8; 3]);
const _: () = assert!(mem::offset_of!(OrderRequest, seq) == 0);
const _: () = assert!(mem::offset_of!(OrderRequest, user_id) == 8);
const _: () = assert!(mem::offset_of!(OrderRequest, symbol_id) == 12);
const _: () = assert!(mem::offset_of!(OrderRequest, price) == 16);
const _: () = assert!(mem::offset_of!(OrderRequest, qty) == 24);
const _: () = assert!(mem::offset_of!(OrderRequest, order_id_hi) == 32);
const _: () = assert!(mem::offset_of!(OrderRequest, order_id_lo) == 40);
const _: () = assert!(mem::offset_of!(OrderRequest, timestamp_ns) == 48);
const _: () = assert!(mem::offset_of!(OrderRequest, side) == 56);
const _: () = assert!(mem::offset_of!(OrderRequest, tif) == 57);
const _: () = assert!(mem::offset_of!(OrderRequest, reduce_only) == 58);
const _: () = assert!(mem::offset_of!(OrderRequest, post_only) == 59);
const _: () = assert!(mem::offset_of!(OrderRequest, is_liquidation) == 60);
const _: () = assert!(mem::offset_of!(OrderRequest, _pad) == 61);

/// Inbound order from the risk engine to a matching engine — the
/// `RECORD_ORDER_REQUEST` payload on the risk→ME hop (risk builds it, ME
/// decodes it, sent via `send_raw`). Per-symbol, so no `symbol_id` and no
/// liquidation flag; risk resolved those upstream.
#[repr(C, align(64))]
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
    pub risk_in_ns: u64,
    pub _pad3: [u8; 56],
}

const _: () = assert!(mem::size_of::<OrderMessage>() == 128);
const _: () = assert!(mem::align_of::<OrderMessage>() == 64);
assert_no_tail_padding!(OrderMessage, _pad3, [u8; 56]);

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
    pub _pad1: [u8; 8],
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
assert_no_tail_padding!(LiquidationRecord, _pad1, [u8; 8]);

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
