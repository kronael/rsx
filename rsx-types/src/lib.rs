//! Fixed-point newtypes, order enums, and hot-thread setup helpers shared by
//! every RSX exchange crate.
//!
//! The leaf crate: it depends on nothing in the project and everything else
//! depends on it. It holds the primitives that need one wire-stable definition
//! across processes — the `#[repr(transparent)]` i64 [`Price`]/[`Qty`] newtypes,
//! the order-lifecycle enums with explicit discriminants, [`SymbolConfig`] +
//! [`validate_order`], and (in the [`cpu`] and [`cache`] modules) the CPU/cache
//! helpers a pinned busy-loop tile needs. No floats, no async runtime, no I/O
//! beyond a single `/sys` read for core-isolation detection. See ARCHITECTURE.md.

pub mod cache;
pub mod cpu;
pub mod macros;
pub mod time_utils;

pub use macros::install_panic_handler;

/// Price in smallest tick units. 1 = one tick.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(transparent)]
pub struct Price(pub i64);

/// Quantity in smallest lot units. 1 = one lot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(transparent)]
pub struct Qty(pub i64);

/// Order side. Discriminants are wire-stable (`Buy = 0`, `Sell = 1`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

/// Time-in-force policy. `GTC` rests; `IOC`/`FOK` are non-resting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TimeInForce {
    GTC = 0,
    IOC = 1,
    FOK = 2,
}

/// Terminal outcome of an order that was accepted (no failure state).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FinalStatus {
    Filled = 0,
    Resting = 1,
    Cancelled = 2,
}

/// Order outcome including the pre-trade `Failed` state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OrderStatus {
    Filled = 0,
    Resting = 1,
    Cancelled = 2,
    Failed = 3,
}

/// Reason an order was rejected. Wire-stable discriminants; surfaced to the
/// client and recorded in the WAL.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FailureReason {
    InvalidTickSize = 0,
    InvalidLotSize = 1,
    SymbolNotFound = 2,
    DuplicateOrderId = 3,
    InsufficientMargin = 4,
    Overloaded = 5,
    InternalError = 6,
    ReduceOnlyViolation = 7,
    NetworkError = 8,
    RateLimit = 9,
    Timeout = 10,
    UserInLiquidation = 11,
    WrongShard = 12,
}

/// Sentinel for "no index" in slab/level linked lists.
pub const NONE: u32 = u32::MAX;

/// Per-symbol tick/lot configuration. `tick_size` and `lot_size` are in raw
/// units; `price_decimals`/`qty_decimals` drive human-readable formatting at
/// the API boundary only.
#[derive(Clone, Debug)]
pub struct SymbolConfig {
    pub symbol_id: u32,
    pub price_decimals: u8,
    pub qty_decimals: u8,
    pub tick_size: i64,
    pub lot_size: i64,
}

/// Order-entry alignment gate: `true` iff `price > 0`, `qty > 0`, and both are
/// exact multiples of the symbol's tick/lot size. The one validation point;
/// the matching engine assumes its inputs already passed here.
pub fn validate_order(config: &SymbolConfig, price: Price, qty: Qty) -> bool {
    price.0 > 0 && qty.0 > 0 && price.0 % config.tick_size == 0 && qty.0 % config.lot_size == 0
}
