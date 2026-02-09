use crate::types::BboUpdate;
use crate::types::FillEvent;
use crate::types::OrderRequest;
use rtrb::Consumer;
use rtrb::Producer;

/// Mark price update from DXS/mark aggregator.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
pub struct MarkPriceUpdate {
    pub symbol_id: u32,
    pub price: i64,
}

/// Response to gateway after pre-trade check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum OrderResponse {
    Accepted {
        user_id: u32,
        margin_reserved: i64,
    },
    Rejected {
        user_id: u32,
        reason: crate::types::RejectReason,
    },
}

/// SPSC ring endpoints for one risk shard.
pub struct ShardRings {
    pub fill_consumers: Vec<Consumer<FillEvent>>,
    pub order_consumer: Consumer<OrderRequest>,
    pub mark_consumer: Consumer<MarkPriceUpdate>,
    pub bbo_consumers: Vec<Consumer<BboUpdate>>,
    pub response_producer: Producer<OrderResponse>,
}
