use rsx_types::Price;
use rsx_types::Qty;

pub const MAX_EVENTS: usize = 10_000;

pub const REASON_FILLED: u8 = 0;
pub const REASON_CANCELLED: u8 = 1;
pub const FAIL_VALIDATION: u8 = 0;
pub const FAIL_REDUCE_ONLY: u8 = 1;
pub const FAIL_FOK: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Fill {
        maker_handle: u32,
        taker_user_id: u32,
        price: Price,
        qty: Qty,
        side: u8,
    },
    OrderInserted {
        handle: u32,
        user_id: u32,
        side: u8,
        price: Price,
        qty: Qty,
    },
    OrderCancelled {
        handle: u32,
        user_id: u32,
        remaining_qty: Qty,
    },
    OrderDone {
        handle: u32,
        user_id: u32,
        reason: u8,
    },
    OrderFailed {
        user_id: u32,
        reason: u8,
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
        }
    }
}
