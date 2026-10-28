use rsx_book::book::Orderbook;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::NONE;

use crate::types::BboUpdate;
use crate::types::L2Delta;
use crate::types::L2Level;
use crate::types::L2Snapshot;
use crate::types::TradeEvent;
use std::collections::HashMap;

pub struct ShadowBook {
    book: Orderbook,
    symbol_id: u32,
    seq: u64,
    timestamp_ns: u64,
    order_map: HashMap<u128, u32>,
}

impl ShadowBook {
    pub fn new(config: SymbolConfig, capacity: u32, mid_price: i64) -> Self {
        let symbol_id = config.symbol_id;
        Self {
            book: Orderbook::new(config, capacity, mid_price),
            symbol_id,
            seq: 0,
            timestamp_ns: 0,
            order_map: HashMap::new(),
        }
    }

    /// Apply a Fill event: reduce maker order qty.
    pub fn apply_fill(&mut self, maker_handle: u32, qty: i64, _side: u8, timestamp_ns: u64) {
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        let slot = self.book.orders.get(maker_handle);
        let remaining = slot.remaining_qty.0;
        let user_id = slot.user_id;
        let order_id_hi = slot.order_id_hi;
        let order_id_lo = slot.order_id_lo;
        let new_qty = remaining - qty;
        if new_qty <= 0 {
            self.book
                .cancel_order_checked(maker_handle, user_id, order_id_hi, order_id_lo);
        } else {
            self.book.modify_order_qty_down(
                maker_handle,
                new_qty,
                user_id,
                order_id_hi,
                order_id_lo,
            );
        }
    }

    /// Apply an OrderInserted event.
    pub fn apply_insert(
        &mut self,
        price: i64,
        qty: i64,
        side: u8,
        user_id: u32,
        timestamp_ns: u64,
    ) -> u32 {
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        let side_enum = if side == 0 { Side::Buy } else { Side::Sell };
        self.book
            .insert_resting(price, qty, side_enum, 0, user_id, false, timestamp_ns, 0, 0)
    }

    /// Apply an OrderCancelled event.
    pub fn apply_cancel(&mut self, handle: u32, timestamp_ns: u64) {
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        self.book.cancel_order(handle);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_insert_by_id(
        &mut self,
        price: i64,
        qty: i64,
        side: u8,
        user_id: u32,
        timestamp_ns: u64,
        order_id_hi: u64,
        order_id_lo: u64,
    ) -> u32 {
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        let side = if side == 0 { Side::Buy } else { Side::Sell };
        let handle = self.book.insert_resting(
            price,
            qty,
            side,
            0,
            user_id,
            false,
            timestamp_ns,
            order_id_hi,
            order_id_lo,
        );
        self.order_map
            .insert(order_key(order_id_hi, order_id_lo), handle);
        handle
    }

    pub fn apply_cancel_by_order_id(
        &mut self,
        order_id_hi: u64,
        order_id_lo: u64,
        timestamp_ns: u64,
    ) -> Option<(u8, i64)> {
        let key = order_key(order_id_hi, order_id_lo);
        let handle = self.order_map.remove(&key)?;
        let slot = self.book.orders.get(handle);
        let side = slot.side;
        let price = slot.price.0;
        let user_id = slot.user_id;
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        self.book
            .cancel_order_checked(handle, user_id, order_id_hi, order_id_lo);
        Some((side, price))
    }

    pub fn apply_fill_by_order_id(
        &mut self,
        order_id_hi: u64,
        order_id_lo: u64,
        qty: i64,
        timestamp_ns: u64,
    ) -> Option<(u8, i64)> {
        let key = order_key(order_id_hi, order_id_lo);
        let handle = *self.order_map.get(&key)?;
        let slot = self.book.orders.get(handle);
        let side = slot.side;
        let price = slot.price.0;
        let user_id = slot.user_id;
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        let remaining = slot.remaining_qty.0;
        let new_qty = remaining - qty;
        if new_qty <= 0 {
            self.book
                .cancel_order_checked(handle, user_id, order_id_hi, order_id_lo);
            self.order_map.remove(&key);
        } else {
            self.book
                .modify_order_qty_down(handle, new_qty, user_id, order_id_hi, order_id_lo);
        }
        Some((side, price))
    }

    /// Recenter the compression map on the current mid once it drifts
    /// past half of zone 0 — mirroring the ME's own `maybe_recenter`
    /// (rsx-matching main.rs). Without this the shadow book stays
    /// anchored at its creation price forever, and a large move pushes
    /// the live ladder into compressed zones where one slot spans many
    /// ticks. Order handles survive migration, so `order_map` stays
    /// valid. Off the order critical path (md dissemination), so the
    /// rare O(book) migration is fine.
    pub fn maybe_recenter(&mut self) {
        if self.book.best_bid_tick == NONE || self.book.best_ask_tick == NONE {
            return;
        }
        let mid = (self.book.best_bid_px + self.book.best_ask_px) / 2;
        if self.book.should_recenter(mid) {
            self.book.recenter_now(mid);
        }
    }

    /// Derive current BBO from the shadow book.
    pub fn derive_bbo(&self) -> Option<BboUpdate> {
        let has_bid = self.book.best_bid_tick != NONE;
        let has_ask = self.book.best_ask_tick != NONE;
        if !has_bid && !has_ask {
            return None;
        }
        let bbo = self.book.current_bbo();
        Some(BboUpdate {
            symbol_id: self.symbol_id,
            bid_px: bbo.bid_px.0,
            bid_qty: bbo.bid_qty.0,
            bid_count: self.count_at(self.book.best_bid_tick, Side::Buy, bbo.bid_px.0),
            ask_px: bbo.ask_px.0,
            ask_qty: bbo.ask_qty.0,
            ask_count: self.count_at(self.book.best_ask_tick, Side::Sell, bbo.ask_px.0),
            timestamp_ns: self.timestamp_ns,
            seq: self.seq,
        })
    }

    /// Generate L2 snapshot up to `depth` levels per side.
    pub fn derive_l2_snapshot(&self, depth: usize) -> L2Snapshot {
        let bids = self.collect_levels_bid(depth);
        let asks = self.collect_levels_ask(depth);
        L2Snapshot {
            symbol_id: self.symbol_id,
            bids,
            asks,
            timestamp_ns: self.timestamp_ns,
            seq: self.seq,
        }
    }

    /// Generate L2 delta for a specific price level.
    pub fn derive_l2_delta(&self, side: u8, price: i64) -> L2Delta {
        let tick = self.book.compression.price_to_index(price);
        let level = &self.book.active_levels[tick as usize];
        L2Delta {
            symbol_id: self.symbol_id,
            side,
            price,
            qty: level.total_qty,
            count: level.order_count,
            timestamp_ns: self.timestamp_ns,
            seq: self.seq,
        }
    }

    /// Build a trade event from fill parameters.
    pub fn make_trade(
        &self,
        price: i64,
        qty: i64,
        taker_side: u8,
        timestamp_ns: u64,
    ) -> TradeEvent {
        TradeEvent {
            symbol_id: self.symbol_id,
            price,
            qty,
            taker_side,
            timestamp_ns,
            seq: self.seq,
        }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn symbol_id(&self) -> u32 {
        self.symbol_id
    }

    fn collect_levels_bid(&self, depth: usize) -> Vec<L2Level> {
        let mut levels = self.side_levels(Side::Buy);
        // Bids best-first = highest price first.
        levels.sort_by_key(|level| std::cmp::Reverse(level.price));
        levels.truncate(depth);
        levels
    }

    fn collect_levels_ask(&self, depth: usize) -> Vec<L2Level> {
        let mut levels = self.side_levels(Side::Sell);
        // Asks best-first = lowest price first.
        levels.sort_by_key(|level| level.price);
        levels.truncate(depth);
        levels
    }

    /// Every non-empty level on one side, unordered. The compression
    /// map is a sawtooth (tick index is not price-monotonic), so the
    /// caller sorts by price rather than walking indices. A compressed
    /// slot can hold both sides and several raw prices, so each slot's
    /// order chain is summed per raw price of the requested side. Off
    /// the order critical path (marketdata dissemination), so the
    /// linear scan + sort is fine.
    fn side_levels(&self, side: Side) -> Vec<L2Level> {
        let mut levels: HashMap<i64, L2Level> = HashMap::new();
        for level in self.book.active_levels.iter() {
            let side_count = match side {
                Side::Buy => level.bid_count,
                Side::Sell => level.ask_count,
            };
            if side_count == 0 {
                continue;
            }
            let mut cursor = level.head;
            while cursor != NONE {
                let order = self.book.orders.get(cursor);
                if order.side == side as u8 {
                    let entry = levels.entry(order.price.0).or_insert(L2Level {
                        price: order.price.0,
                        qty: 0,
                        count: 0,
                    });
                    entry.qty += order.remaining_qty.0;
                    entry.count += 1;
                }
                cursor = order.next;
            }
        }
        levels.into_values().collect()
    }

    fn count_at(&self, tick: u32, side: Side, price: i64) -> u32 {
        if tick == NONE {
            return 0;
        }
        let mut count = 0;
        let mut cursor = self.book.active_levels[tick as usize].head;
        while cursor != NONE {
            let order = self.book.orders.get(cursor);
            if order.side == side as u8 && order.price.0 == price {
                count += 1;
            }
            cursor = order.next;
        }
        count
    }
}

fn order_key(order_id_hi: u64, order_id_lo: u64) -> u128 {
    ((order_id_hi as u128) << 64) | order_id_lo as u128
}

#[cfg(test)]
#[path = "shadow_test.rs"]
mod shadow_test;
