use rsx_cast::cast::CastSender;
use rsx_messages::CancelRequest;
use rsx_messages::OrderRequest;
use rtrb::Producer;
use tracing::warn;

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

/// A command for the ME that owns the symbol. `Order` carries the
/// accepted order and its risk ingress timestamp.
#[derive(Clone, Copy, Debug)]
pub enum MeCommand {
    Order(OrderRequest, u64),
    Cancel(CancelRequest),
}

/// Egress SPSC ring endpoints for one risk shard. The recv loop
/// calls `process_*` directly (same thread); these two carry the
/// shard's output back to the live loop's casting senders.
pub struct ShardRings {
    pub response_producer: Producer<OrderResponse>,
    pub me_producer: Producer<MeCommand>,
}

/// Forward a record onto risk's gateway stream, renumbering it
/// with `gw`'s own contiguous seq (SEQ-1 fix). Risk's gateway
/// stream multiplexes forwarded ME records AND risk-generated
/// margin rejects; preserving ME's seq (or the reject's seq=0)
/// leaves holes the gateway reads as FAULTED, and seq=0 records
/// are dropped outright by the receiver. The gateway never
/// replays *from* risk, so renumbering is safe — the seq is
/// transport-only on this hop; the record is identified by its
/// order_id. CRC is recomputed by `send_raw` over the restamped
/// payload.
pub fn forward_to_gw(gw: &mut CastSender, record_type: u16, payload: &[u8]) {
    let plen = payload.len();
    if !(8..=256).contains(&plen) {
        warn!("risk: gw forward bad payload len={plen}");
        return;
    }
    let mut buf = [0u8; 256];
    buf[..plen].copy_from_slice(payload);
    let seq = gw.next_seq();
    buf[0..8].copy_from_slice(&seq.to_le_bytes());
    if let Err(e) = gw.send_raw(record_type, &buf[..plen]) {
        warn!("risk: forward to gw failed: {e}");
    }
    gw.advance_seq();
}
