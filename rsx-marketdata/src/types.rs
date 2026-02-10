#[derive(Debug, Clone, PartialEq)]
pub struct BboUpdate {
    pub symbol_id: u32,
    pub bid_px: i64,
    pub bid_qty: i64,
    pub bid_count: u32,
    pub ask_px: i64,
    pub ask_qty: i64,
    pub ask_count: u32,
    pub timestamp_ns: u64,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct L2Level {
    pub price: i64,
    pub qty: i64,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct L2Snapshot {
    pub symbol_id: u32,
    pub bids: Vec<L2Level>,
    pub asks: Vec<L2Level>,
    pub timestamp_ns: u64,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct L2Delta {
    pub symbol_id: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
    pub count: u32,
    pub timestamp_ns: u64,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TradeEvent {
    pub symbol_id: u32,
    pub price: i64,
    pub qty: i64,
    pub taker_side: u8,
    pub timestamp_ns: u64,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarketDataMessage {
    Bbo(BboUpdate),
    Snapshot(L2Snapshot),
    Delta(L2Delta),
    Trade(TradeEvent),
}
