/// Gateway -> Risk: new order request
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct RiskNewOrder {
    pub order_id: [u8; 16],
    pub client_order_id: [u8; 20],
    pub user_id: u32,
    pub symbol_id: u32,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: u8,
    pub post_only: u8,
    pub is_liquidation: u8,
    pub _pad: [u8; 3],
    pub price: i64,
    pub qty: i64,
    pub timestamp_ns: u64,
}

/// Gateway -> Risk: cancel order request
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct RiskCancelOrder {
    pub user_id: u32,
    pub symbol_id: u32,
    pub order_id: [u8; 16],
    pub client_order_id: [u8; 20],
    pub cancel_by_oid: bool,
    pub _pad: [u8; 3],
    pub timestamp_ns: u64,
}

/// Risk -> Gateway: order status update
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct RiskOrderUpdate {
    pub order_id: [u8; 16],
    pub client_order_id: [u8; 20],
    pub user_id: u32,
    pub symbol_id: u32,
    pub status: u8,
    pub reason: u8,
    pub _pad: [u8; 6],
    pub filled_qty: i64,
    pub remaining_qty: i64,
}

/// Risk -> Gateway: fill notification
#[repr(C, align(64))]
#[derive(Clone, Debug)]
pub struct OrderFill {
    pub taker_order_id: [u8; 16],
    pub maker_order_id: [u8; 16],
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub price: i64,
    pub qty: i64,
    pub taker_side: u8,
    pub _pad: [u8; 7],
    pub timestamp_ns: u64,
    pub taker_fee: i64,
    pub maker_fee: i64,
}

/// Risk -> Gateway: stream error
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct StreamError {
    pub code: u32,
    pub _pad: u32,
    pub msg: [u8; 56],
}
