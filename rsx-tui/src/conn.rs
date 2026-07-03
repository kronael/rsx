//! Gateway transport abstraction.
//!
//! The TUI talks to the exchange through a `GatewayConn`: it submits
//! orders and drains inbound events non-blocking each render tick. The
//! real implementation is a QUIC client (webproto 49) — added behind
//! this trait so the whole UI can be built and tested against a
//! `MockConn` with no network. Casting (the internal GW↔Risk↔ME
//! transport) is unrelated and untouched; QUIC is user-facing only.

use std::collections::VecDeque;
use std::io;

/// A side for an order or a print.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Buy,
    Sell,
}

/// Time-in-force.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tif {
    Gtc,
    Ioc,
    Fok,
}

impl Tif {
    pub fn label(self) -> &'static str {
        match self {
            Tif::Gtc => "GTC",
            Tif::Ioc => "IOC",
            Tif::Fok => "FOK",
        }
    }

    /// Cycle GTC -> IOC -> FOK -> GTC.
    pub fn next(self) -> Tif {
        match self {
            Tif::Gtc => Tif::Ioc,
            Tif::Ioc => Tif::Fok,
            Tif::Fok => Tif::Gtc,
        }
    }
}

/// An order the UI wants to submit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OrderReq {
    pub side: Side,
    pub price: i64,
    pub qty: i64,
    pub tif: Tif,
}

/// Inbound event from the gateway. The UI folds these into `App`
/// state each tick. Prices/quantities are raw i64 fixed-point, the
/// wire representation (conversion to human units is a display
/// concern, done at render).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GwEvent {
    Connected,
    Disconnected,
    /// Full L2 ladder snapshot: bids best-first, asks best-first.
    Book {
        bids: Vec<(i64, i64)>,
        asks: Vec<(i64, i64)>,
    },
    /// A public trade print.
    Trade {
        side: Side,
        px: i64,
        qty: i64,
    },
    /// The gateway accepted an order we submitted.
    Accepted {
        oid: u64,
    },
    /// A fill against one of our orders.
    Fill {
        oid: u64,
        px: i64,
        qty: i64,
        side: Side,
    },
    /// An order reached a terminal state (done or cancelled).
    Done {
        oid: u64,
    },
    /// The gateway rejected an order (pre-trade / margin / malformed).
    Rejected {
        reason: String,
    },
    /// A position update for the account.
    Position {
        symbol: &'static str,
        net_qty: i64,
        entry_px: i64,
        upnl: i64,
    },
}

/// Transport to the gateway. Non-blocking: `poll_event` returns the
/// next queued event or `None`. The UI calls it in a loop each render
/// tick until it drains.
pub trait GatewayConn {
    /// Queue an order for submission. Errors if the link is down.
    fn submit(&mut self, order: OrderReq) -> io::Result<()>;
    /// Next inbound event, or `None` if nothing is pending.
    fn poll_event(&mut self) -> Option<GwEvent>;
}

/// In-memory transport for tests and offline demo: a scripted queue
/// of inbound events, and a record of orders the UI submitted.
pub struct MockConn {
    inbound: VecDeque<GwEvent>,
    pub submitted: Vec<OrderReq>,
    /// When true, `submit` fails (models a dropped link).
    pub down: bool,
}

impl MockConn {
    pub fn new() -> Self {
        MockConn {
            inbound: VecDeque::new(),
            submitted: Vec::new(),
            down: false,
        }
    }

    /// Queue events the UI will observe on subsequent polls.
    pub fn push_events(&mut self, events: impl IntoIterator<Item = GwEvent>) {
        self.inbound.extend(events);
    }
}

impl Default for MockConn {
    fn default() -> Self {
        MockConn::new()
    }
}

impl GatewayConn for MockConn {
    fn submit(&mut self, order: OrderReq) -> io::Result<()> {
        if self.down {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "gateway link down",
            ));
        }
        self.submitted.push(order);
        Ok(())
    }

    fn poll_event(&mut self) -> Option<GwEvent> {
        self.inbound.pop_front()
    }
}
