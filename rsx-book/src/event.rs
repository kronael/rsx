use rsx_types::Price;
use rsx_types::Qty;

/// Educational per-order event cap. `emit` panics rather than dropping;
/// production needs a resumable match-and-drain boundary.
pub const MAX_EVENTS: usize = 65_536;

pub const REASON_FILLED: u8 = 0;
pub const REASON_CANCELLED: u8 = 1;
pub const FAIL_VALIDATION: u8 = 0;
pub const FAIL_REDUCE_ONLY: u8 = 1;
pub const FAIL_FOK: u8 = 2;
/// Duplicate order (ME dedup). Detected in `rsx-matching` before the book,
/// but its `OrderFailedRecord.reason` code lives in this one namespace so
/// no future `FAIL_*` value silently aliases it.
pub const FAIL_DUPLICATE: u8 = 3;

pub const CANCEL_USER: u8 = 0;
pub const CANCEL_REDUCE_ONLY: u8 = 1;
pub const CANCEL_IOC: u8 = 2;
pub const CANCEL_POST_ONLY: u8 = 3;

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Fill {
        maker_handle: u32,
        maker_tif: u8,
        maker_reduce_only: bool,
        maker_user_id: u32,
        taker_user_id: u32,
        price: Price,
        qty: Qty,
        side: u8,
        maker_order_id_hi: u64,
        maker_order_id_lo: u64,
        taker_order_id_hi: u64,
        taker_order_id_lo: u64,
        /// Taker order's gateway-ingress timestamp, echoed
        /// onto FillRecord.gw_in_ns so the internal/engine
        /// per-hop deltas (specs/2/59-latency-observability.md)
        /// anchor against the same t0 as risk_in / me_in.
        gw_in_ns: u64,
    },
    OrderInserted {
        handle: u32,
        user_id: u32,
        side: u8,
        price: Price,
        qty: Qty,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderCancelled {
        handle: u32,
        user_id: u32,
        remaining_qty: Qty,
        reason: u8,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderDone {
        handle: u32,
        tif: u8,
        reduce_only: bool,
        user_id: u32,
        reason: u8,
        filled_qty: Qty,
        remaining_qty: Qty,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderFailed {
        user_id: u32,
        reason: u8,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    BBO {
        bid_px: Price,
        bid_qty: Qty,
        ask_px: Price,
        ask_qty: Qty,
    },
}

impl Default for Event {
    fn default() -> Self {
        Event::OrderFailed {
            user_id: 0,
            reason: 0,
            order_id_hi: 0,
            order_id_lo: 0,
        }
    }
}
