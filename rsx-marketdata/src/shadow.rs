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
    pub fn new(
        config: SymbolConfig,
        capacity: u32,
        mid_price: i64,
    ) -> Self {
        let symbol_id = config.symbol_id;
        Self {
            book: Orderbook::new(
                config, capacity, mid_price,
            ),
            symbol_id,
            seq: 0,
            timestamp_ns: 0,
            order_map: HashMap::new(),
        }
    }

    /// Apply a Fill event: reduce maker order qty.
    pub fn apply_fill(
        &mut self,
        maker_handle: u32,
        qty: i64,
        _side: u8,
        timestamp_ns: u64,
    ) {
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        let remaining =
            self.book.orders.get(maker_handle)
                .remaining_qty.0;
        let new_qty = remaining - qty;
        if new_qty <= 0 {
            self.book.cancel_order(maker_handle);
        } else {
            self.book.modify_order_qty_down(
                maker_handle, new_qty,
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
        let side_enum = if side == 0 {
            Side::Buy
        } else {
            Side::Sell
        };
        self.book.insert_resting(
            price, qty, side_enum, 0, user_id, false,
            timestamp_ns, 0, 0,
        )
    }

    /// Apply an OrderCancelled event.
    pub fn apply_cancel(
        &mut self,
        handle: u32,
        timestamp_ns: u64,
    ) {
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        self.book.cancel_order(handle);
    }

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
        let handle = self.apply_insert(
            price,
            qty,
            side,
            user_id,
            timestamp_ns,
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
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        self.book.cancel_order(handle);
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
        self.seq += 1;
        self.timestamp_ns = timestamp_ns;
        let remaining = slot.remaining_qty.0;
        let new_qty = remaining - qty;
        if new_qty <= 0 {
            self.book.cancel_order(handle);
            self.order_map.remove(&key);
        } else {
            self.book.modify_order_qty_down(handle, new_qty);
        }
        Some((side, price))
    }

    /// Derive current BBO from the shadow book.
    pub fn derive_bbo(&self) -> Option<BboUpdate> {
        let has_bid = self.book.best_bid_tick != NONE;
        let has_ask = self.book.best_ask_tick != NONE;
        if !has_bid && !has_ask {
            return None;
        }
        let (bid_px, bid_qty, bid_count) = if has_bid {
            let tick = self.book.best_bid_tick;
            let level =
                &self.book.active_levels[tick as usize];
            let head = level.head;
            let price =
                self.book.orders.get(head).price.0;
            (price, level.total_qty, level.order_count)
        } else {
            (0, 0, 0)
        };
        let (ask_px, ask_qty, ask_count) = if has_ask {
            let tick = self.book.best_ask_tick;
            let level =
                &self.book.active_levels[tick as usize];
            let head = level.head;
            let price =
                self.book.orders.get(head).price.0;
            (price, level.total_qty, level.order_count)
        } else {
            (0, 0, 0)
        };
        Some(BboUpdate {
            symbol_id: self.symbol_id,
            bid_px,
            bid_qty,
            bid_count,
            ask_px,
            ask_qty,
            ask_count,
            timestamp_ns: self.timestamp_ns,
            seq: self.seq,
        })
    }

    /// Generate L2 snapshot up to `depth` levels per side.
    pub fn derive_l2_snapshot(
        &self,
        depth: usize,
    ) -> L2Snapshot {
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
    pub fn derive_l2_delta(
        &self,
        side: u8,
        price: i64,
    ) -> L2Delta {
        let tick =
            self.book.compression.price_to_index(price);
        let level =
            &self.book.active_levels[tick as usize];
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

    fn collect_levels_bid(
        &self,
        depth: usize,
    ) -> Vec<L2Level> {
        let mut levels = Vec::with_capacity(depth);
        let mut tick = self.book.best_bid_tick;
        while tick != NONE && levels.len() < depth {
            let level =
                &self.book.active_levels[tick as usize];
            if level.order_count > 0 {
                let head = level.head;
                let price =
                    self.book.orders.get(head).price.0;
                levels.push(L2Level {
                    price,
                    qty: level.total_qty,
                    count: level.order_count,
                });
            }
            tick = self.book.scan_next_bid(tick);
        }
        levels
    }

    fn collect_levels_ask(
        &self,
        depth: usize,
    ) -> Vec<L2Level> {
        let mut levels = Vec::with_capacity(depth);
        let mut tick = self.book.best_ask_tick;
        while tick != NONE && levels.len() < depth {
            let level =
                &self.book.active_levels[tick as usize];
            if level.order_count > 0 {
                let head = level.head;
                let price =
                    self.book.orders.get(head).price.0;
                levels.push(L2Level {
                    price,
                    qty: level.total_qty,
                    count: level.order_count,
                });
            }
            tick = self.book.scan_next_ask(tick);
        }
        levels
    }
}

fn order_key(order_id_hi: u64, order_id_lo: u64) -> u128 {
    ((order_id_hi as u128) << 64) | order_id_lo as u128
}
