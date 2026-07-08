//! Protobuf-over-QUIC frame codec.
//!
//! Length-delimited frames: a 4-byte big-endian length prefix followed
//! by a protobuf (prost) body. Client→server frames carry a `WireOrder`;
//! server→client frames carry a `WireEvent` — a `oneof` mirroring the
//! subset of `GwEvent` that actually crosses the wire (`Connected` /
//! `Disconnected` are synthesized locally by `QuicConn`, never sent).
//!
//! The schema is deliberately minimal: exactly the fields the TUI submits
//! (`OrderReq`) and renders (`GwEvent`), nothing speculative. The types
//! are hand-derived `prost::Message`/`prost::Oneof` structs — no `.proto`
//! file, no `prost-build`, no `protoc` at build time.
//!
//! Byte-level interop with a real gateway is a follow-up: no gateway
//! speaks this protobuf-over-QUIC wire yet, so it is exercised against a
//! loopback QUIC server (`tests/quic_test.rs`). The message set here is
//! the contract that server would implement.

use crate::conn::GwEvent;
use crate::conn::OrderReq;
use crate::conn::Side;
use crate::conn::Tif;
use prost::Message;
use quinn::RecvStream;
use quinn::SendStream;
use std::io;

/// Reject frames larger than this (guards a corrupt/hostile length
/// prefix from triggering a huge allocation). 1 MiB is far above any
/// legitimate order or event frame.
const MAX_FRAME: usize = 1 << 20;

/// Map any error carrying a message into an `io::Error`.
fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

// --- protobuf schema (exactly what the TUI sends/renders) ---

/// Client→server order submission. `side`/`tif` are small ints (0-based)
/// rather than proto enums to keep the schema flat; the mapping lives in
/// the conversions below.
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireOrder {
    #[prost(int32, tag = "1")]
    side: i32,
    #[prost(int64, tag = "2")]
    price: i64,
    #[prost(int64, tag = "3")]
    qty: i64,
    #[prost(int32, tag = "4")]
    tif: i32,
}

/// One price level in an L2 snapshot.
#[derive(Clone, PartialEq, ::prost::Message)]
struct BookLevel {
    #[prost(int64, tag = "1")]
    px: i64,
    #[prost(int64, tag = "2")]
    qty: i64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Book {
    #[prost(message, repeated, tag = "1")]
    bids: Vec<BookLevel>,
    #[prost(message, repeated, tag = "2")]
    asks: Vec<BookLevel>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Trade {
    #[prost(int32, tag = "1")]
    side: i32,
    #[prost(int64, tag = "2")]
    px: i64,
    #[prost(int64, tag = "3")]
    qty: i64,
}

/// An order id alone — carries `Accepted` and `Done`.
#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderId {
    #[prost(uint64, tag = "1")]
    oid: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Fill {
    #[prost(uint64, tag = "1")]
    oid: u64,
    #[prost(int64, tag = "2")]
    px: i64,
    #[prost(int64, tag = "3")]
    qty: i64,
    #[prost(int32, tag = "4")]
    side: i32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Rejected {
    #[prost(string, tag = "1")]
    reason: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Position {
    #[prost(string, tag = "1")]
    symbol: String,
    #[prost(int64, tag = "2")]
    net_qty: i64,
    #[prost(int64, tag = "3")]
    entry_px: i64,
    #[prost(int64, tag = "4")]
    upnl: i64,
}

/// Server-stamped latency breakdown. `net_ns` is optional: the gateway
/// leaves it unset (the client fills the net leg from its measured RTT).
#[derive(Clone, PartialEq, ::prost::Message)]
struct Latency {
    #[prost(uint64, optional, tag = "1")]
    net_ns: Option<u64>,
    #[prost(uint64, tag = "2")]
    internal_ns: u64,
    #[prost(uint64, tag = "3")]
    engine_ns: u64,
}

/// The server→client event body: one of the wire event kinds.
#[derive(Clone, PartialEq, ::prost::Oneof)]
enum Event {
    #[prost(message, tag = "1")]
    Book(Book),
    #[prost(message, tag = "2")]
    Trade(Trade),
    #[prost(message, tag = "3")]
    Accepted(OrderId),
    #[prost(message, tag = "4")]
    Fill(Fill),
    #[prost(message, tag = "5")]
    Done(OrderId),
    #[prost(message, tag = "6")]
    Rejected(Rejected),
    #[prost(message, tag = "7")]
    Position(Position),
    #[prost(message, tag = "8")]
    Latency(Latency),
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct WireEvent {
    #[prost(oneof = "Event", tags = "1,2,3,4,5,6,7,8")]
    event: Option<Event>,
}

// --- domain <-> wire conversions ---

fn side_to_i32(side: Side) -> i32 {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

fn side_from_i32(v: i32) -> Side {
    match v {
        1 => Side::Sell,
        _ => Side::Buy,
    }
}

fn tif_to_i32(tif: Tif) -> i32 {
    match tif {
        Tif::Gtc => 0,
        Tif::Ioc => 1,
        Tif::Fok => 2,
    }
}

fn tif_from_i32(v: i32) -> Tif {
    match v {
        1 => Tif::Ioc,
        2 => Tif::Fok,
        _ => Tif::Gtc,
    }
}

impl From<&OrderReq> for WireOrder {
    fn from(o: &OrderReq) -> Self {
        WireOrder {
            side: side_to_i32(o.side),
            price: o.price,
            qty: o.qty,
            tif: tif_to_i32(o.tif),
        }
    }
}

impl From<WireOrder> for OrderReq {
    fn from(w: WireOrder) -> Self {
        OrderReq {
            side: side_from_i32(w.side),
            price: w.price,
            qty: w.qty,
            tif: tif_from_i32(w.tif),
        }
    }
}

/// Map a `GwEvent` to its wire form. Errors on `Connected`/`Disconnected`
/// — those are synthesized by `QuicConn`, never transmitted.
fn to_wire_event(ev: &GwEvent) -> io::Result<WireEvent> {
    let event = match ev {
        GwEvent::Book { bids, asks } => Event::Book(Book {
            bids: bids
                .iter()
                .map(|&(px, qty)| BookLevel { px, qty })
                .collect(),
            asks: asks
                .iter()
                .map(|&(px, qty)| BookLevel { px, qty })
                .collect(),
        }),
        GwEvent::Trade { side, px, qty } => Event::Trade(Trade {
            side: side_to_i32(*side),
            px: *px,
            qty: *qty,
        }),
        GwEvent::Accepted { oid } => Event::Accepted(OrderId { oid: *oid }),
        GwEvent::Fill { oid, px, qty, side } => Event::Fill(Fill {
            oid: *oid,
            px: *px,
            qty: *qty,
            side: side_to_i32(*side),
        }),
        GwEvent::Done { oid } => Event::Done(OrderId { oid: *oid }),
        GwEvent::Rejected { reason } => Event::Rejected(Rejected {
            reason: reason.clone(),
        }),
        GwEvent::Position {
            symbol,
            net_qty,
            entry_px,
            upnl,
        } => Event::Position(Position {
            symbol: symbol.clone(),
            net_qty: *net_qty,
            entry_px: *entry_px,
            upnl: *upnl,
        }),
        GwEvent::Latency {
            net_ns,
            internal_ns,
            engine_ns,
        } => Event::Latency(Latency {
            net_ns: *net_ns,
            internal_ns: *internal_ns,
            engine_ns: *engine_ns,
        }),
        GwEvent::Connected | GwEvent::Disconnected => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Connected/Disconnected are local events, not wire frames",
            ));
        }
    };
    Ok(WireEvent { event: Some(event) })
}

fn from_wire_event(w: WireEvent) -> io::Result<GwEvent> {
    let event = w
        .event
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "event frame carries no kind"))?;
    let ev = match event {
        Event::Book(b) => GwEvent::Book {
            bids: b.bids.into_iter().map(|l| (l.px, l.qty)).collect(),
            asks: b.asks.into_iter().map(|l| (l.px, l.qty)).collect(),
        },
        Event::Trade(t) => GwEvent::Trade {
            side: side_from_i32(t.side),
            px: t.px,
            qty: t.qty,
        },
        Event::Accepted(o) => GwEvent::Accepted { oid: o.oid },
        Event::Fill(f) => GwEvent::Fill {
            oid: f.oid,
            px: f.px,
            qty: f.qty,
            side: side_from_i32(f.side),
        },
        Event::Done(o) => GwEvent::Done { oid: o.oid },
        Event::Rejected(r) => GwEvent::Rejected { reason: r.reason },
        Event::Position(p) => GwEvent::Position {
            symbol: p.symbol,
            net_qty: p.net_qty,
            entry_px: p.entry_px,
            upnl: p.upnl,
        },
        Event::Latency(l) => GwEvent::Latency {
            net_ns: l.net_ns,
            internal_ns: l.internal_ns,
            engine_ns: l.engine_ns,
        },
    };
    Ok(ev)
}

// --- framing ---

async fn write_frame(send: &mut SendStream, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    send.write_all(&len.to_be_bytes()).await.map_err(to_io)?;
    send.write_all(body).await.map_err(to_io)?;
    Ok(())
}

async fn read_frame(recv: &mut RecvStream) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(to_io)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(to_io)?;
    Ok(body)
}

/// Client→server: encode and send one order frame.
pub async fn write_order(send: &mut SendStream, order: &OrderReq) -> io::Result<()> {
    let body = WireOrder::from(order).encode_to_vec();
    write_frame(send, &body).await
}

/// Server→client on the read side (or a test server): decode one order
/// frame.
pub async fn read_order(recv: &mut RecvStream) -> io::Result<OrderReq> {
    let body = read_frame(recv).await?;
    let order = WireOrder::decode(&body[..]).map_err(to_io)?;
    Ok(order.into())
}

/// Server→client: encode and send one event frame.
pub async fn write_event(send: &mut SendStream, ev: &GwEvent) -> io::Result<()> {
    let body = to_wire_event(ev)?.encode_to_vec();
    write_frame(send, &body).await
}

/// Client side: decode one event frame into a `GwEvent`.
pub async fn read_event(recv: &mut RecvStream) -> io::Result<GwEvent> {
    let body = read_frame(recv).await?;
    let event = WireEvent::decode(&body[..]).map_err(to_io)?;
    from_wire_event(event)
}
