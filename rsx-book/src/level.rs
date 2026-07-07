use rsx_types::NONE;

#[derive(Clone, Copy, Debug)]
pub struct PriceLevel {
    pub head: u32,
    pub tail: u32,
    pub total_qty: i64,
    pub order_count: u32,
    /// Resting orders of each side in this slot. In compressed zones a
    /// slot can hold BOTH sides (and multiple raw prices), so per-side
    /// occupancy and BBA correctness cannot be derived from a single
    /// `order_count` / the FIFO head — `bid_count > 0` iff ≥1 buy rests
    /// here, `ask_count > 0` iff ≥1 sell. `order_count == bid_count +
    /// ask_count`. Zone 0 is 1:1 (single price → single side), so one of
    /// the two is always 0 there.
    pub bid_count: u32,
    pub ask_count: u32,
}

const _: () = assert!(std::mem::size_of::<PriceLevel>() == 32);

impl Default for PriceLevel {
    fn default() -> Self {
        Self {
            head: NONE,
            tail: NONE,
            total_qty: 0,
            order_count: 0,
            bid_count: 0,
            ask_count: 0,
        }
    }
}
