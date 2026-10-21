use rsx_messages::BboRecord;
use rsx_messages::FillRecord;

/// A fill off the ME cast stream, the part risk applies. RISK.md §1.
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

/// RISK.md §6: reject reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    InsufficientMargin,
    UserInLiquidation,
    NotInShard,
    SymbolNotConfigured,
}

/// A BBO off the ME cast stream. RISK.md §4.
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

impl From<&FillRecord> for FillEvent {
    fn from(fill: &FillRecord) -> Self {
        Self {
            seq: fill.seq,
            symbol_id: fill.symbol_id,
            taker_user_id: fill.taker_user_id,
            maker_user_id: fill.maker_user_id,
            price: fill.price.0,
            qty: fill.qty.0,
            taker_side: fill.taker_side,
            timestamp_ns: fill.ts_ns,
        }
    }
}

impl From<&BboRecord> for BboUpdate {
    fn from(bbo: &BboRecord) -> Self {
        Self {
            seq: bbo.seq,
            symbol_id: bbo.symbol_id,
            bid_px: bbo.bid_px.0,
            bid_qty: bbo.bid_qty.0,
            ask_px: bbo.ask_px.0,
            ask_qty: bbo.ask_qty.0,
        }
    }
}
