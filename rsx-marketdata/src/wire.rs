//! Protobuf codec for the public market-data feed (server -> subscriber).
//!
//! Each feed message is encoded as one `MdFrame` (a `oneof` over the
//! five message kinds) and sent as a WebSocket BINARY frame, framed as
//! a 4-byte big-endian length prefix followed by the prost body:
//! `[len:u32 BE][MdFrame protobuf bytes]`.
//!
//! WebSocket already delimits messages, so the length prefix is redundant
//! on this transport, but it is kept so the frame format stays
//! transport-agnostic.
//!
//! The schema is a hand-derived `prost::Message`/`prost::Oneof` set — no
//! `.proto`, no `prost-build`, no `protoc` on the build path. The shared
//! schema is documented in `rsx-marketdata/marketdata.proto`. The encoders
//! and [`decode`] live here and share one set of wire structs, so the
//! decoder is the exact inverse of the encoder. `wire_test.rs` pins the
//! exact encoded bytes so an accidental tag or field change fails a test,
//! and round-trips every frame kind.
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

#[derive(Clone, PartialEq, ::prost::Message)]
struct WireLevel {
    #[prost(int64, tag = "1")]
    px: i64,
    #[prost(int64, tag = "2")]
    qty: i64,
    #[prost(uint32, tag = "3")]
    count: u32,
}

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

#[derive(Clone, PartialEq, ::prost::Message)]
struct WireHeartbeat {
    #[prost(uint64, tag = "1")]
    timestamp_ms: u64,
}

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

#[derive(Clone, PartialEq, ::prost::Message)]
struct MdFrame {
    #[prost(oneof = "Body", tags = "1,2,3,4,5")]
    body: Option<Body>,
}

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

/// One decoded feed message. `Delta::side` is 0=bid, 1=ask and a delta
/// `qty` of 0 removes the level; `TradeEvent::taker_side` is 0=buy, 1=sell.
/// A scalar the sender omitted (proto3 default) decodes as 0.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Bbo(BboUpdate),
    Snapshot(L2Snapshot),
    Delta(L2Delta),
    Trade(TradeEvent),
    Heartbeat { timestamp_ms: u64 },
}

/// Why [`decode`] rejected a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer than the 4 length-prefix bytes.
    Truncated { len: usize },
    /// The length prefix differs from the number of body bytes.
    LengthMismatch { prefix: usize, body: usize },
    /// The body is not a valid `MdFrame`.
    Protobuf(prost::DecodeError),
    /// The envelope carries none of the five message kinds.
    EmptyFrame,
    /// A delta side or trade taker side that does not fit a `u8`.
    SideOutOfRange(u32),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Truncated { len } => {
                write!(
                    f,
                    "frame of {len} bytes is shorter than its 4-byte length prefix"
                )
            }
            DecodeError::LengthMismatch { prefix, body } => {
                write!(f, "length prefix {prefix} but body is {body} bytes")
            }
            DecodeError::Protobuf(error) => write!(f, "invalid MdFrame: {error}"),
            DecodeError::EmptyFrame => write!(f, "MdFrame carries no message"),
            DecodeError::SideOutOfRange(side) => write!(f, "side {side} is not a u8"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode one length-prefixed `MdFrame`, the inverse of the `encode_*`
/// functions: `decode(&encode_x(&x)) == Ok(x)` for every value. The input
/// is exactly one frame (one WebSocket BINARY message). Fields this schema
/// does not define are skipped; when a oneof repeats, the last one wins.
pub fn decode(frame: &[u8]) -> Result<Frame, DecodeError> {
    let Some((prefix, body)) = frame.split_first_chunk::<4>() else {
        return Err(DecodeError::Truncated { len: frame.len() });
    };
    let prefix = u32::from_be_bytes(*prefix) as usize;
    if prefix != body.len() {
        return Err(DecodeError::LengthMismatch {
            prefix,
            body: body.len(),
        });
    }
    let envelope = MdFrame::decode(body).map_err(DecodeError::Protobuf)?;
    match envelope.body.ok_or(DecodeError::EmptyFrame)? {
        Body::Bbo(bbo) => Ok(Frame::Bbo(BboUpdate {
            symbol_id: bbo.symbol_id,
            bid_px: bbo.bid_px,
            bid_qty: bbo.bid_qty,
            bid_count: bbo.bid_count,
            ask_px: bbo.ask_px,
            ask_qty: bbo.ask_qty,
            ask_count: bbo.ask_count,
            timestamp_ns: bbo.timestamp_ns,
            seq: bbo.seq,
        })),
        Body::Snapshot(snap) => Ok(Frame::Snapshot(L2Snapshot {
            symbol_id: snap.symbol_id,
            bids: snap.bids.into_iter().map(level_of).collect(),
            asks: snap.asks.into_iter().map(level_of).collect(),
            timestamp_ns: snap.timestamp_ns,
            seq: snap.seq,
        })),
        Body::Delta(delta) => Ok(Frame::Delta(L2Delta {
            symbol_id: delta.symbol_id,
            side: u8::try_from(delta.side).map_err(|_| DecodeError::SideOutOfRange(delta.side))?,
            price: delta.price,
            qty: delta.qty,
            count: delta.count,
            timestamp_ns: delta.timestamp_ns,
            seq: delta.seq,
        })),
        Body::Trade(trade) => Ok(Frame::Trade(TradeEvent {
            symbol_id: trade.symbol_id,
            price: trade.price,
            qty: trade.qty,
            taker_side: u8::try_from(trade.taker_side)
                .map_err(|_| DecodeError::SideOutOfRange(trade.taker_side))?,
            timestamp_ns: trade.timestamp_ns,
            seq: trade.seq,
        })),
        Body::Heartbeat(heartbeat) => Ok(Frame::Heartbeat {
            timestamp_ms: heartbeat.timestamp_ms,
        }),
    }
}

fn level_of(level: WireLevel) -> L2Level {
    L2Level {
        price: level.px,
        qty: level.qty,
        count: level.count,
    }
}

#[cfg(test)]
#[path = "wire_test.rs"]
mod wire_test;
