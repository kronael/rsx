use rsx_types::Price;
use rsx_types::Qty;

/// Per-order event cap. `emit` panics rather than dropping; the match loop
/// stops at `MAX_MAKERS_PER_ORDER`, so the panic is unreachable.
pub const MAX_EVENTS: usize = 65_536;

/// Makers one order may match. Each costs at most a Fill and an OrderDone;
/// the taker's OrderDone and a BBO use the rest of `MAX_EVENTS`.
pub const MAX_MAKERS_PER_ORDER: usize = (MAX_EVENTS - 4) / 2;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderDoneReason {
    Filled = 0,
    UserCancel = 1,
    Expired = 2,
    PostOnly = 3,
    MatchLimit = 4,
}

/// Book-side failure reason. The discriminants are NOT the wire codes: the ME
/// maps each variant to `rsx_types::FailureReason` when it builds the record.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderFailureReason {
    InvalidTick = 0,
    ReduceOnly = 1,
    FillOrKill = 2,
    Duplicate = 3,
    InvalidLot = 4,
}

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
        /// Gateway-ingress timestamp used as the end-to-end latency origin.
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
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderDone {
        handle: u32,
        tif: u8,
        reduce_only: bool,
        user_id: u32,
        reason: OrderDoneReason,
        filled_qty: Qty,
        remaining_qty: Qty,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderFailed {
        user_id: u32,
        reason: OrderFailureReason,
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
