//! Protobuf-over-QUIC frame codec.
//!
//! Length-delimited frames: a 4-byte big-endian length prefix followed
//! by a protobuf (prost) body. The stream is:
//!
//! - **first frame, client→server:** a `WireHello` carrying the session
//!   JWT + user id — the auth first-frame. The gateway MUST validate it
//!   before accepting any order (that server-side validation is a
//!   follow-up, not built here); the client sends identity in-band
//!   instead of connecting anonymously.
//! - **then, client→server:** `WireOrder` frames — each carries a client
//!   correlation id (`cid`) and the `symbol` it trades. The TUI is
//!   single-market, so `symbol` is a per-session constant, not a field
//!   of the UI's `OrderReq`.
//! - **server→client:** `WireEvent` frames — a `oneof` mirroring the
//!   subset of `GwEvent` that crosses the wire (`Connected` /
//!   `Disconnected` are synthesized locally by `QuicConn`, never sent).
//!   A `Latency` event echoes the order's `cid` so the client pairs the
//!   sample to the submitted order (not FIFO-guess) and fills the net leg.
//!
//! The schema is deliberately minimal and hand-derived `prost::Message`/
//! `prost::Oneof` structs — no `.proto`, no `prost-build`, no `protoc`.
//! `wire_test.rs` pins the exact encoded bytes of each message so an
//! accidental tag/field change fails a test.
//!
//! No gateway speaks this wire yet, so it is exercised against a loopback
//! QUIC server (`tests/quic_test.rs`) — the reference for what a real
//! gateway endpoint would implement.

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

/// Auth first-frame (client→server): the session JWT and the user id it
/// claims. Sent once, before any order. The gateway validates the JWT
/// (server-side follow-up) and binds the connection to `user`.
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireHello {
    #[prost(string, tag = "1")]
    jwt: String,
    #[prost(uint32, tag = "2")]
    user: u32,
}

/// Client→server order submission. `cid` is the client correlation id
/// (echoed on `Latency`); `symbol` is the instrument (the TUI trades
/// one). `side`/`tif` are small ints (0-based) rather than proto enums
/// to keep the schema flat; the mapping lives in the conversions below.
#[derive(Clone, PartialEq, ::prost::Message)]
struct WireOrder {
    #[prost(uint64, tag = "1")]
    cid: u64,
    #[prost(uint32, tag = "2")]
    symbol: u32,
    #[prost(int32, tag = "3")]
    side: i32,
    #[prost(int64, tag = "4")]
    price: i64,
    #[prost(int64, tag = "5")]
    qty: i64,
    #[prost(int32, tag = "6")]
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

/// Server-stamped latency breakdown. `cid` echoes the submitting order's
/// client correlation id so the client pairs the sample to that order.
/// `net_ns` is optional: the gateway leaves it unset (the client fills
/// the net leg from its measured RTT).
#[derive(Clone, PartialEq, ::prost::Message)]
struct Latency {
    #[prost(uint64, tag = "1")]
    cid: u64,
    #[prost(uint64, optional, tag = "2")]
    net_ns: Option<u64>,
    #[prost(uint64, tag = "3")]
    internal_ns: u64,
    #[prost(uint64, tag = "4")]
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

/// A decoded auth first-frame, as the server reads it.
pub struct Hello {
    pub jwt: String,
    pub user: u32,
}

/// A decoded client order, as the server reads it: the correlation id
/// and symbol the client stamped, plus the order fields.
pub struct IncomingOrder {
    pub cid: u64,
    pub symbol: u32,
    pub order: OrderReq,
}

fn side_to_i32(side: Side) -> i32 {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

/// Decode a wire side. Errors — never coerces — on an unknown value, so
/// a garbled or future-versioned frame fails the read instead of
/// silently trading the wrong direction.
fn side_from_i32(v: i32) -> io::Result<Side> {
    match v {
        0 => Ok(Side::Buy),
        1 => Ok(Side::Sell),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown side {other}"),
        )),
    }
}

fn tif_to_i32(tif: Tif) -> i32 {
    match tif {
        Tif::Gtc => 0,
        Tif::Ioc => 1,
        Tif::Fok => 2,
    }
}

/// Decode a wire time-in-force. Errors — never coerces — on an unknown
/// value (see `side_from_i32`).
fn tif_from_i32(v: i32) -> io::Result<Tif> {
    match v {
        0 => Ok(Tif::Gtc),
        1 => Ok(Tif::Ioc),
        2 => Ok(Tif::Fok),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown tif {other}"),
        )),
    }
}

impl TryFrom<WireOrder> for OrderReq {
    type Error = io::Error;

    fn try_from(w: WireOrder) -> io::Result<Self> {
        Ok(OrderReq {
            side: side_from_i32(w.side)?,
            price: w.price,
            qty: w.qty,
            tif: tif_from_i32(w.tif)?,
        })
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
            cid,
            net_ns,
            internal_ns,
            engine_ns,
        } => Event::Latency(Latency {
            cid: *cid,
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
            side: side_from_i32(t.side)?,
            px: t.px,
            qty: t.qty,
        },
        Event::Accepted(o) => GwEvent::Accepted { oid: o.oid },
        Event::Fill(f) => GwEvent::Fill {
            oid: f.oid,
            px: f.px,
            qty: f.qty,
            side: side_from_i32(f.side)?,
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
            cid: l.cid,
            net_ns: l.net_ns,
            internal_ns: l.internal_ns,
            engine_ns: l.engine_ns,
        },
    };
    Ok(ev)
}

// --- framing ---

async fn write_frame(send: &mut SendStream, body: &[u8]) -> io::Result<()> {
    // Reject an oversized body on write with the same bound the reader
    // enforces, so a peer never emits a frame its own reader would drop.
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length exceeds MAX_FRAME",
        ));
    }
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

/// Client→server: send the auth first-frame (JWT + user id). Sent once,
/// before any order frame.
pub async fn write_hello(send: &mut SendStream, jwt: &str, user: u32) -> io::Result<()> {
    let body = WireHello {
        jwt: jwt.to_owned(),
        user,
    }
    .encode_to_vec();
    write_frame(send, &body).await
}

/// Server→client on the read side (or a test server): decode the auth
/// first-frame.
pub async fn read_hello(recv: &mut RecvStream) -> io::Result<Hello> {
    let body = read_frame(recv).await?;
    let hello = WireHello::decode(&body[..]).map_err(to_io)?;
    Ok(Hello {
        jwt: hello.jwt,
        user: hello.user,
    })
}

/// Client→server: encode and send one order frame, stamping the client
/// correlation id and the session symbol.
pub async fn write_order(
    send: &mut SendStream,
    symbol: u32,
    cid: u64,
    order: &OrderReq,
) -> io::Result<()> {
    let body = WireOrder {
        cid,
        symbol,
        side: side_to_i32(order.side),
        price: order.price,
        qty: order.qty,
        tif: tif_to_i32(order.tif),
    }
    .encode_to_vec();
    write_frame(send, &body).await
}

/// Server→client on the read side (or a test server): decode one order
/// frame with its correlation id and symbol.
pub async fn read_order(recv: &mut RecvStream) -> io::Result<IncomingOrder> {
    let body = read_frame(recv).await?;
    let w = WireOrder::decode(&body[..]).map_err(to_io)?;
    let cid = w.cid;
    let symbol = w.symbol;
    Ok(IncomingOrder {
        cid,
        symbol,
        order: OrderReq::try_from(w)?,
    })
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

#[cfg(test)]
#[path = "wire_test.rs"]
mod wire_test;
