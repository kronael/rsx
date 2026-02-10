/// From ME SPSC ring. RISK.md §1.
#[derive(Clone, Debug)]
#[repr(C, align(64))]
pub struct FillEvent {
    pub seq: u64,
    pub symbol_id: u32,
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub price: i64,
    pub qty: i64,
    pub taker_side: u8,
    pub timestamp_ns: u64,
}

/// From Gateway SPSC ring. RISK.md §6.
#[derive(Clone, Debug)]
#[repr(C, align(64))]
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
    pub is_liquidation: bool,
    pub _pad: [u8; 4],
}

/// RISK.md §6: reject reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    InsufficientMargin,
    UserInLiquidation,
    NotInShard,
}

/// From ME SPSC ring. RISK.md §4.
#[derive(Clone, Debug)]
#[repr(C, align(64))]
pub struct BboUpdate {
    pub seq: u64,
    pub symbol_id: u32,
    pub bid_px: i64,
    pub bid_qty: i64,
    pub ask_px: i64,
    pub ask_qty: i64,
}

/// From ME: order completed or failed. Releases frozen
/// margin.
#[derive(Clone, Debug)]
#[repr(C, align(64))]
pub struct OrderDoneEvent {
    pub seq: u64,
    pub user_id: u32,
    pub symbol_id: u32,
    pub frozen_amount: i64,
}
