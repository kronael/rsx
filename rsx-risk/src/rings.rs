use crate::types::OrderRequest;
use rtrb::Producer;

/// Response to gateway after pre-trade check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum OrderResponse {
    Accepted {
        user_id: u32,
        margin_reserved: i64,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    Rejected {
        user_id: u32,
        reason: crate::types::RejectReason,
        order_id_hi: u64,
        order_id_lo: u64,
    },
}

/// Egress SPSC ring endpoints for one risk shard. Input rings
/// removed: the recv loop calls `process_*` directly (same
/// thread). These two carry the shard's output back to the
/// main loop's casting senders.
pub struct ShardRings {
    pub response_producer: Producer<OrderResponse>,
    pub accepted_producer: Producer<(OrderRequest, u64)>,
}
