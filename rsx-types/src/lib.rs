/// Price in smallest tick units. 1 = one tick.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
    Hash, Debug, Default,
)]
#[repr(transparent)]
pub struct Price(pub i64);

/// Quantity in smallest lot units. 1 = one lot.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
    Hash, Debug, Default,
)]
#[repr(transparent)]
pub struct Qty(pub i64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TimeInForce {
    GTC = 0,
    IOC = 1,
    FOK = 2,
}

/// Sentinel for "no index" in slab/level linked lists.
pub const NONE: u32 = u32::MAX;

pub type SlabIdx = u32;

#[derive(Clone, Debug)]
pub struct SymbolConfig {
    pub symbol_id: u32,
    pub price_decimals: u8,
    pub qty_decimals: u8,
    pub tick_size: i64,
    pub lot_size: i64,
}

pub fn validate_order(
    config: &SymbolConfig,
    price: Price,
    qty: Qty,
) -> bool {
    price.0 > 0
        && qty.0 > 0
        && price.0 % config.tick_size == 0
        && qty.0 % config.lot_size == 0
}
