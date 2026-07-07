//! UI state and the event-fold that keeps it in sync with the gateway.
//!
//! `App` is pure state; `apply_event` folds one `GwEvent` into it and
//! `drain` pumps the transport each tick. Nothing here touches a
//! terminal or a socket, so a full session can be driven from tests.

use crate::conn::GatewayConn;
use crate::conn::GwEvent;
use crate::conn::OrderReq;
use crate::conn::Side;
use crate::conn::Tif;
use std::collections::VecDeque;

/// Trade tape depth.
const MAX_TRADES: usize = 50;

/// Rolling latency-sample window (for p50 / min).
const MAX_LAT: usize = 128;

/// One measured round-trip, split by where the time went (ns). `net_ns`
/// is `None` when the client couldn't pair the report to a submitted
/// order (so the display shows "—", never a fabricated 0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Latency {
    /// client ↔ gateway (QUIC leg); `None` if unmeasured.
    pub net_ns: Option<u64>,
    /// casting GW→Risk→ME→Risk→GW (internal UDP).
    pub internal_ns: u64,
    /// ME match + risk processing.
    pub engine_ns: u64,
}

impl Latency {
    /// Total round-trip. Counts the net leg only when it was measured.
    pub fn total_ns(&self) -> u64 {
        self.net_ns
            .unwrap_or(0)
            .saturating_add(self.internal_ns)
            .saturating_add(self.engine_ns)
    }

    /// True once every leg (incl. net) is known — a complete sample.
    pub fn is_complete(&self) -> bool {
        self.net_ns.is_some()
    }
}

/// Which order-entry field has focus (digits/backspace edit it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Price,
    Qty,
}

/// Editable order-entry form. Price/qty are digit buffers so typing is
/// explicit and testable; `submit_order` parses them.
pub struct OrderEntry {
    pub side: Side,
    pub price: String,
    pub qty: String,
    pub tif: Tif,
    pub focus: Field,
}

impl OrderEntry {
    fn new() -> Self {
        OrderEntry {
            side: Side::Buy,
            price: String::new(),
            qty: String::new(),
            tif: Tif::Gtc,
            focus: Field::Price,
        }
    }

    fn buf_mut(&mut self) -> &mut String {
        match self.focus {
            Field::Price => &mut self.price,
            Field::Qty => &mut self.qty,
        }
    }

    /// A well-formed order, or `None` if either field is empty/zero.
    pub fn to_order(&self) -> Option<OrderReq> {
        let price: i64 = self.price.parse().ok()?;
        let qty: i64 = self.qty.parse().ok()?;
        if price <= 0 || qty <= 0 {
            return None;
        }
        Some(OrderReq {
            side: self.side,
            price,
            qty,
            tif: self.tif,
        })
    }
}

/// Terminal UI state.
pub struct App {
    pub symbol: &'static str,
    /// Bid levels, best (highest) price first.
    pub bids: Vec<(i64, i64)>,
    /// Ask levels, best (lowest) price first.
    pub asks: Vec<(i64, i64)>,
    /// Recent prints, newest first.
    pub trades: VecDeque<(Side, i64, i64)>,
    /// (symbol, net_qty, entry_px, upnl). Owned symbol (no leak).
    pub positions: Vec<(String, i64, i64, i64)>,
    pub connected: bool,
    /// Last event, shown in the status bar.
    pub status: String,
    pub entry: OrderEntry,
    /// Live (accepted, not yet done) order count.
    pub open_orders: usize,
    /// Fills observed this session.
    pub fills: usize,
    /// Most recent measured round-trip breakdown.
    pub last_lat: Option<Latency>,
    /// Rolling window of round-trip totals (ns) for p50 / min.
    pub lat_totals: VecDeque<u64>,
    /// Gateway endpoint (for the header + trace overlay).
    pub endpoint: String,
    /// Trace overlay toggle (F3) — a diagnostic HUD over the UI.
    pub show_trace: bool,
}

impl App {
    /// Empty app for a live session (before any gateway event).
    pub fn new(symbol: &'static str) -> Self {
        App {
            symbol,
            bids: Vec::new(),
            asks: Vec::new(),
            trades: VecDeque::new(),
            positions: Vec::new(),
            connected: false,
            status: "connecting…".to_owned(),
            entry: OrderEntry::new(),
            open_orders: 0,
            fills: 0,
            last_lat: None,
            lat_totals: VecDeque::new(),
            endpoint: String::new(),
            show_trace: false,
        }
    }

    /// Record the gateway endpoint (shown in the header + trace HUD).
    pub fn set_endpoint(&mut self, url: &str) {
        self.endpoint = url.to_owned();
    }

    /// p50 of the round-trip totals in the rolling window (ns).
    pub fn lat_p50_ns(&self) -> Option<u64> {
        if self.lat_totals.is_empty() {
            return None;
        }
        let mut v: Vec<u64> = self.lat_totals.iter().copied().collect();
        v.sort_unstable();
        Some(v[v.len() / 2])
    }

    /// Minimum round-trip total in the rolling window (ns).
    pub fn lat_min_ns(&self) -> Option<u64> {
        self.lat_totals.iter().copied().min()
    }

    /// Best ask minus best bid; 0 if either side is empty.
    pub fn spread(&self) -> i64 {
        let best_ask = self.asks.first().map(|a| a.0).unwrap_or(0);
        let best_bid = self.bids.first().map(|b| b.0).unwrap_or(0);
        if best_ask == 0 || best_bid == 0 {
            return 0;
        }
        best_ask - best_bid
    }

    /// Fold one inbound event into state.
    pub fn apply_event(&mut self, ev: GwEvent) {
        match ev {
            GwEvent::Connected => {
                self.connected = true;
                self.status = "connected".to_owned();
            }
            GwEvent::Disconnected => {
                self.connected = false;
                self.status = "disconnected".to_owned();
            }
            GwEvent::Book { bids, asks } => {
                self.bids = bids;
                self.asks = asks;
            }
            GwEvent::Trade { side, px, qty } => {
                self.trades.push_front((side, px, qty));
                self.trades.truncate(MAX_TRADES);
            }
            GwEvent::Accepted { oid } => {
                self.open_orders += 1;
                self.status = format!("order {oid} accepted");
            }
            GwEvent::Fill { oid, px, qty, .. } => {
                self.fills += 1;
                self.status = format!("fill {oid}: {qty} @ {px}");
            }
            GwEvent::Done { oid } => {
                self.open_orders = self.open_orders.saturating_sub(1);
                self.status = format!("order {oid} done");
            }
            GwEvent::Rejected { reason } => {
                self.status = format!("rejected: {reason}");
            }
            GwEvent::Position {
                symbol,
                net_qty,
                entry_px,
                upnl,
            } => match self.positions.iter_mut().find(|p| p.0 == symbol) {
                Some(p) => {
                    p.1 = net_qty;
                    p.2 = entry_px;
                    p.3 = upnl;
                }
                None => self.positions.push((symbol, net_qty, entry_px, upnl)),
            },
            GwEvent::Latency {
                net_ns,
                internal_ns,
                engine_ns,
            } => {
                let lat = Latency {
                    net_ns,
                    internal_ns,
                    engine_ns,
                };
                // Only complete samples (net known) drive p50 / best.
                if lat.is_complete() {
                    self.lat_totals.push_back(lat.total_ns());
                    if self.lat_totals.len() > MAX_LAT {
                        self.lat_totals.pop_front();
                    }
                }
                self.last_lat = Some(lat);
            }
        }
    }

    // --- order-entry editing (called by input.rs) ---

    pub fn input_digit(&mut self, c: char) {
        if c.is_ascii_digit() {
            self.entry.buf_mut().push(c);
        }
    }

    pub fn input_backspace(&mut self) {
        self.entry.buf_mut().pop();
    }

    pub fn toggle_focus(&mut self) {
        self.entry.focus = match self.entry.focus {
            Field::Price => Field::Qty,
            Field::Qty => Field::Price,
        };
    }

    pub fn set_side(&mut self, side: Side) {
        self.entry.side = side;
    }

    pub fn cycle_tif(&mut self) {
        self.entry.tif = self.entry.tif.next();
    }

    /// Submit the current form over `conn`. Returns whether an order
    /// went out (false = incomplete form or link error, with `status`
    /// updated to say why).
    pub fn submit_order(&mut self, conn: &mut dyn GatewayConn) -> bool {
        let Some(order) = self.entry.to_order() else {
            self.status = "incomplete order (need price & qty)".to_owned();
            return false;
        };
        match conn.submit(order) {
            Ok(()) => {
                self.status = format!(
                    "sent {:?} {} @ {} [{}]",
                    order.side,
                    order.qty,
                    order.price,
                    order.tif.label(),
                );
                self.entry.price.clear();
                self.entry.qty.clear();
                self.entry.focus = Field::Price;
                true
            }
            Err(e) => {
                self.status = format!("submit failed: {e}");
                false
            }
        }
    }
}

/// Drain every pending event from the transport into the app. Called
/// once per render tick.
pub fn drain(app: &mut App, conn: &mut dyn GatewayConn) {
    while let Some(ev) = conn.poll_event() {
        app.apply_event(ev);
    }
}
