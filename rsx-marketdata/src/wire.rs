//! Protobuf codec for the public market-data feed (server -> subscriber).
//!
//! Each feed message is encoded as one `MdFrame` (a `oneof` over the
//! five message kinds) and sent as a WebSocket BINARY frame, framed as
//! a 4-byte big-endian length prefix followed by the prost body:
//! `[len:u32 BE][MdFrame protobuf bytes]`.
//!
//! The length prefix mirrors `rsx-tui/src/wire.rs`. WebSocket already
//! delimits messages, so the prefix is redundant on this transport, but
//! it is kept for parity and to keep the frame format transport-agnostic.
//!
//! The schema is a hand-derived `prost::Message`/`prost::Oneof` set — no
//! `.proto`, no `prost-build`, no `protoc` on the build path. The shared
//! schema is documented in `rsx-marketdata/marketdata.proto`; the Python
//! subscriber (`rsx-playground/md_wire.py`) hand-rolls a matching decoder.
//! `wire_test.rs` pins the exact encoded bytes so an accidental tag or
//! field change fails a test.
//!
//! Note: `MdFrame` here is the OUTBOUND feed envelope. The inbound
//! client control frame (subscribe/unsubscribe/heartbeat) is the
//! separate `records::MdFrame`, parsed from JSON text — the two never
//! meet on the wire and are never imported together.

use crate::types::BboUpdate;
use crate::types::L2Delta;
use crate::types::L2Level;
use crate::types::L2Snapshot;
use crate::types::TradeEvent;
use prost::Message;

// --- protobuf schema (matches marketdata.proto; tags are STABLE) ---

/// One aggregated price level (`Level` in the .proto).
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireLevel {
    #[prost(int64, tag = "1")]
    px: i64,
    #[prost(int64, tag = "2")]
    qty: i64,
    #[prost(uint32, tag = "3")]
    count: u32,
}

/// Best bid/offer (`Bbo`).
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireBbo {
    #[prost(uint32, tag = "1")]
    symbol_id: u32,
    #[prost(int64, tag = "2")]
    bid_px: i64,
    #[prost(int64, tag = "3")]
    bid_qty: i64,
    #[prost(uint32, tag = "4")]
    bid_count: u32,
    #[prost(int64, tag = "5")]
    ask_px: i64,
    #[prost(int64, tag = "6")]
    ask_qty: i64,
    #[prost(uint32, tag = "7")]
    ask_count: u32,
    #[prost(uint64, tag = "8")]
    timestamp_ns: u64,
    #[prost(uint64, tag = "9")]
    seq: u64,
}

/// Full L2 snapshot (`Snapshot`).
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireSnapshot {
    #[prost(uint32, tag = "1")]
    symbol_id: u32,
    #[prost(message, repeated, tag = "2")]
    bids: Vec<WireLevel>,
    #[prost(message, repeated, tag = "3")]
    asks: Vec<WireLevel>,
    #[prost(uint64, tag = "4")]
    timestamp_ns: u64,
    #[prost(uint64, tag = "5")]
    seq: u64,
}

/// L2 delta for a single level (`Delta`). `side`: 0=bid, 1=ask; `qty`
/// == 0 removes the level.
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireDelta {
    #[prost(uint32, tag = "1")]
    symbol_id: u32,
    #[prost(uint32, tag = "2")]
    side: u32,
    #[prost(int64, tag = "3")]
    price: i64,
    #[prost(int64, tag = "4")]
    qty: i64,
    #[prost(uint32, tag = "5")]
    count: u32,
    #[prost(uint64, tag = "6")]
    timestamp_ns: u64,
    #[prost(uint64, tag = "7")]
    seq: u64,
}

/// A trade print (`Trade`). `taker_side`: 0=buy, 1=sell.
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireTrade {
    #[prost(uint32, tag = "1")]
    symbol_id: u32,
    #[prost(int64, tag = "2")]
    price: i64,
    #[prost(int64, tag = "3")]
    qty: i64,
    #[prost(uint32, tag = "4")]
    taker_side: u32,
    #[prost(uint64, tag = "5")]
    timestamp_ns: u64,
    #[prost(uint64, tag = "6")]
    seq: u64,
}

/// Server->client liveness ping (`Heartbeat`).
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireHeartbeat {
    #[prost(uint64, tag = "1")]
    timestamp_ms: u64,
}

/// The feed body: exactly one message kind per frame.
#[derive(Clone, PartialEq, ::prost::Oneof)]
enum Body {
    #[prost(message, tag = "1")]
    Bbo(WireBbo),
    #[prost(message, tag = "2")]
    Snapshot(WireSnapshot),
    #[prost(message, tag = "3")]
    Delta(WireDelta),
    #[prost(message, tag = "4")]
    Trade(WireTrade),
    #[prost(message, tag = "5")]
    Heartbeat(WireHeartbeat),
}

/// Outbound feed envelope.
#[derive(Clone, PartialEq, ::prost::Message)]
struct MdFrame {
    #[prost(oneof = "Body", tags = "1,2,3,4,5")]
    body: Option<Body>,
}

// --- framing ---

/// Encode `body` into an `MdFrame` and prepend the 4-byte big-endian
/// length prefix. Panics only on the impossible >4GiB frame.
fn frame(body: Body) -> Vec<u8> {
    let payload = MdFrame { body: Some(body) }.encode_to_vec();
    let len =
        u32::try_from(payload.len()).expect("INVARIANT: MdFrame body never exceeds u32::MAX bytes");
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

fn wire_level(level: &L2Level) -> WireLevel {
    WireLevel {
        px: level.price,
        qty: level.qty,
        count: level.count,
    }
}

// --- domain -> wire encoders (the send path) ---

/// Encode a BBO update to a length-prefixed `MdFrame` frame.
pub fn encode_bbo(bbo: &BboUpdate) -> Vec<u8> {
    frame(Body::Bbo(WireBbo {
        symbol_id: bbo.symbol_id,
        bid_px: bbo.bid_px,
        bid_qty: bbo.bid_qty,
        bid_count: bbo.bid_count,
        ask_px: bbo.ask_px,
        ask_qty: bbo.ask_qty,
        ask_count: bbo.ask_count,
        timestamp_ns: bbo.timestamp_ns,
        seq: bbo.seq,
    }))
}

/// Encode an L2 snapshot to a length-prefixed `MdFrame` frame.
pub fn encode_l2_snapshot(snap: &L2Snapshot) -> Vec<u8> {
    frame(Body::Snapshot(WireSnapshot {
        symbol_id: snap.symbol_id,
        bids: snap.bids.iter().map(wire_level).collect(),
        asks: snap.asks.iter().map(wire_level).collect(),
        timestamp_ns: snap.timestamp_ns,
        seq: snap.seq,
    }))
}

/// Encode an L2 delta to a length-prefixed `MdFrame` frame.
pub fn encode_l2_delta(delta: &L2Delta) -> Vec<u8> {
    frame(Body::Delta(WireDelta {
        symbol_id: delta.symbol_id,
        side: delta.side as u32,
        price: delta.price,
        qty: delta.qty,
        count: delta.count,
        timestamp_ns: delta.timestamp_ns,
        seq: delta.seq,
    }))
}

/// Encode a trade print to a length-prefixed `MdFrame` frame.
pub fn encode_trade(trade: &TradeEvent) -> Vec<u8> {
    frame(Body::Trade(WireTrade {
        symbol_id: trade.symbol_id,
        price: trade.price,
        qty: trade.qty,
        taker_side: trade.taker_side as u32,
        timestamp_ns: trade.timestamp_ns,
        seq: trade.seq,
    }))
}

/// Encode a server->client heartbeat to a length-prefixed `MdFrame` frame.
pub fn encode_heartbeat(timestamp_ms: u64) -> Vec<u8> {
    frame(Body::Heartbeat(WireHeartbeat { timestamp_ms }))
}

#[cfg(test)]
#[path = "wire_test.rs"]
mod wire_test;
