use rsx_types::NONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rustc_hash::FxHashMap;

use crate::compression::CompressionMap;
use crate::event::Event;
use crate::event::MAX_EVENTS;
use crate::level::PriceLevel;
use crate::order::OrderSlot;
use crate::slab::Slab;
use crate::user::UserState;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BookState {
    Normal,
    Migrating,
}

pub struct Orderbook {
    pub active_levels: Vec<PriceLevel>,
    pub staging_levels: Vec<PriceLevel>,
    pub orders: Slab<OrderSlot>,
    pub best_bid_tick: u32,
    pub best_ask_tick: u32,
    /// Raw price of the best bid/ask level. The compression map is a
    /// SAWTOOTH (index is not globally price-monotonic across zones),
    /// so BBA must be tracked/compared by raw price, never by tick
    /// index. NONE tick => price is 0 (unused).
    pub best_bid_px: i64,
    pub best_ask_px: i64,
    pub compression: CompressionMap,
    pub state: BookState,
    pub config: SymbolConfig,
    pub sequence: u64,
    /// Heap-boxed: at 65_536 events the inline array would
    /// overflow the stack during `Orderbook::new`. Heap
    /// allocation is fine since this happens once at
    /// startup, not on the hot path.
    pub event_buf: Box<[Event; MAX_EVENTS]>,
    pub event_len: u32,
    // User position tracking
    pub user_states: Vec<UserState>,
    pub user_map: FxHashMap<u32, u16>,
    pub user_free_list: Vec<u16>,
    pub user_bump: u16,
    // Migration state
    pub old_levels: Option<Vec<PriceLevel>>,
    pub old_compression: Option<CompressionMap>,
    pub bid_frontier: i64,
    pub ask_frontier: i64,
    pub old_min_price: i64,
    pub old_max_price: i64,
}

impl Orderbook {
    pub fn new(
        config: SymbolConfig,
        capacity: u32,
        mid_price: i64,
    ) -> Self {
        let compression =
            CompressionMap::new(mid_price, config.tick_size);
        let total = compression.total_slots() as usize;
        let active_levels =
            vec![PriceLevel::default(); total];
        let staging_levels =
            vec![PriceLevel::default(); total];
        Self {
            active_levels,
            staging_levels,
            orders: Slab::new(capacity),
            best_bid_tick: NONE,
            best_ask_tick: NONE,
            best_bid_px: 0,
            best_ask_px: 0,
            compression,
            state: BookState::Normal,
            config,
            sequence: 0,
            event_buf: vec![Event::default(); MAX_EVENTS]
                .into_boxed_slice()
                .try_into()
                .expect(
                    "INVARIANT: event_buf must have \
                     exactly MAX_EVENTS slots",
                ),
            event_len: 0,
            user_states: Vec::with_capacity(256),
            user_map: FxHashMap::default(),
            user_free_list: Vec::new(),
            user_bump: 0,
            old_levels: None,
            old_compression: None,
            bid_frontier: mid_price,
            ask_frontier: mid_price,
            old_min_price: 0,
            old_max_price: 0,
        }
    }

    /// Invariant: ME never drops events (see spec
    /// `Correctness Invariants`). `MAX_EVENTS` is sized
    /// to accommodate the worst-case cascade for one
    /// order; overflow indicates a runaway cascade and
    /// is treated as an unrecoverable bug.
    #[inline]
    pub fn emit(&mut self, event: Event) {
        assert!(
            (self.event_len as usize) < MAX_EVENTS,
            "INVARIANT: ME event buffer overflow \
             (MAX_EVENTS={}); runaway cascade",
            MAX_EVENTS,
        );
        self.event_buf[self.event_len as usize] = event;
        self.event_len += 1;
    }

    pub fn events(&self) -> &[Event] {
        &self.event_buf[..self.event_len as usize]
    }

    pub fn is_migrating(&self) -> bool {
        self.state == BookState::Migrating
    }

    pub fn update_config(&mut self, new_config: SymbolConfig) {
        self.config = new_config;
    }

    /// Insert a resting order into the book.
    ///
    /// Invariant #3 (FIFO within price level): new orders link at
    /// `level.tail`; `match_at_level` walks from `level.head`, so
    /// time priority is preserved per price level.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_resting(
        &mut self,
        price: i64,
        qty: i64,
        side: Side,
        tif: u8,
        user_id: u32,
        reduce_only: bool,
        timestamp_ns: u64,
        order_id_hi: u64,
        order_id_lo: u64,
    ) -> u32 {
        let tick =
            self.compression.price_to_index(price);
        let handle = self.orders.alloc();
        let slot = self.orders.get_mut(handle);
        slot.price = Price(price);
        slot.remaining_qty = Qty(qty);
        slot.original_qty = Qty(qty);
        slot.side = side as u8;
        slot.tif = tif;
        slot.flags = 1; // active
        if reduce_only {
            slot.flags |= 2;
        }
        slot.user_id = user_id;
        slot.tick_index = tick;
        slot.timestamp_ns = timestamp_ns;
        slot.order_id_hi = order_id_hi;
        slot.order_id_lo = order_id_lo;
        slot.next = NONE;
        slot.prev = NONE;
        self.sequence += 1;
        slot.sequence = self.sequence as u32;

        // Link into level
        let level =
            &mut self.active_levels[tick as usize];
        if level.order_count == 0 {
            level.head = handle;
            level.tail = handle;
        } else {
            let old_tail = level.tail;
            self.orders.get_mut(old_tail).next = handle;
            self.orders.get_mut(handle).prev = old_tail;
            level.tail = handle;
        }
        level.total_qty += qty;
        level.order_count += 1;

        // Update BBA by RAW PRICE (compression index is a sawtooth,
        // not a price proxy — see best_bid_px doc).
        match side {
            Side::Buy => {
                if self.best_bid_tick == NONE
                    || price > self.best_bid_px
                {
                    self.best_bid_tick = tick;
                    self.best_bid_px = price;
                }
            }
            Side::Sell => {
                if self.best_ask_tick == NONE
                    || price < self.best_ask_px
                {
                    self.best_ask_tick = tick;
                    self.best_ask_px = price;
                }
            }
        }

        // Track user order count
        let uidx = crate::user::get_or_assign_user(
            &mut self.user_states,
            &mut self.user_map,
            &mut self.user_free_list,
            &mut self.user_bump,
            user_id,
        );
        self.user_states[uidx as usize].order_count
            += 1;

        handle
    }

    /// Unlink an order from its level's doubly-linked
    /// list. Adjusts head/tail, total_qty, order_count.
    /// Does not free the slab slot.
    pub fn unlink_order(&mut self, handle: u32) {
        let slot = self.orders.get(handle);
        let tick = slot.tick_index;
        let qty = slot.remaining_qty.0;
        let prev = slot.prev;
        let next = slot.next;
        let level =
            &mut self.active_levels[tick as usize];
        if prev != NONE {
            self.orders.get_mut(prev).next = next;
        } else {
            level.head = next;
        }
        if next != NONE {
            self.orders.get_mut(next).prev = prev;
        } else {
            level.tail = prev;
        }
        level.total_qty =
            level.total_qty.saturating_sub(qty);
        level.order_count =
            level.order_count.saturating_sub(1);
    }

    /// Cancel an order by slab handle.
    pub fn cancel_order(&mut self, handle: u32) -> bool {
        let slot = self.orders.get(handle);
        if !slot.is_active() {
            return false;
        }
        let tick = slot.tick_index;
        let side = slot.side;
        let user_id = slot.user_id;

        self.unlink_order(handle);

        // Update BBA if needed
        if self.active_levels[tick as usize].order_count == 0 {
            if side == Side::Buy as u8
                && tick == self.best_bid_tick
            {
                self.best_bid_tick =
                    self.scan_next_bid(tick);
                self.best_bid_px = self
                    .price_at_tick(self.best_bid_tick);
            } else if side == Side::Sell as u8
                && tick == self.best_ask_tick
            {
                self.best_ask_tick =
                    self.scan_next_ask(tick);
                self.best_ask_px = self
                    .price_at_tick(self.best_ask_tick);
            }
        }

        self.orders.get_mut(handle).set_active(false);
        self.orders.free(handle);

        // Decrement user order count
        if let Some(&uidx) =
            self.user_map.get(&user_id)
        {
            self.user_states[uidx as usize]
                .order_count =
                self.user_states[uidx as usize]
                    .order_count
                    .saturating_sub(1);
        }

        true
    }

    /// Modify order price: cancel at old price, reinsert
    /// at new price. Returns new slab handle. Loses time
    /// priority (new order at back of queue).
    #[allow(clippy::too_many_arguments)]
    pub fn modify_order_price(
        &mut self,
        handle: u32,
        new_price: i64,
        side: Side,
        tif: u8,
        user_id: u32,
        reduce_only: bool,
        timestamp_ns: u64,
        order_id_hi: u64,
        order_id_lo: u64,
    ) -> u32 {
        let qty = self.orders.get(handle).remaining_qty.0;
        self.cancel_order(handle);
        self.insert_resting(
            new_price, qty, side, tif, user_id,
            reduce_only, timestamp_ns,
            order_id_hi, order_id_lo,
        )
    }

    /// Reduce remaining qty in-place. Preserves time
    /// priority. If new_qty == 0, cancels the order.
    /// Returns true if order was active.
    pub fn modify_order_qty_down(
        &mut self,
        handle: u32,
        new_qty: i64,
    ) -> bool {
        let slot = self.orders.get(handle);
        if !slot.is_active() {
            return false;
        }
        if new_qty == 0 {
            return self.cancel_order(handle);
        }
        let old_qty = slot.remaining_qty.0;
        if new_qty >= old_qty {
            return false; // not a reduction
        }
        let tick = slot.tick_index;
        let diff = old_qty - new_qty;
        self.orders.get_mut(handle).remaining_qty =
            Qty(new_qty);
        self.active_levels[tick as usize].total_qty =
            self.active_levels[tick as usize]
                .total_qty
                .saturating_sub(diff);
        true
    }

    /// Raw price of the head order at `tick`, or 0 if `tick == NONE`.
    #[inline]
    pub fn price_at_tick(&self, tick: u32) -> i64 {
        if tick == NONE {
            return 0;
        }
        let head = self.active_levels[tick as usize].head;
        if head == NONE {
            return 0;
        }
        self.orders.get(head).price.0
    }

    /// Find the tick of the highest-priced non-empty bid level.
    ///
    /// The compression map is a sawtooth: tick index is NOT globally
    /// price-monotonic, so we cannot walk indices to find the next-best
    /// bid. Instead do a single bounded linear pass over the fixed slot
    /// array and take the max price among BUY levels. The just-vacated
    /// best level (`_from`) is excluded implicitly (its `order_count` is
    /// 0). No allocation. O(slots) worst case, but only runs when the
    /// best bid level empties (cancel or full consumption), not per
    /// order. `_from` is unused, kept for call-site symmetry.
    pub fn scan_next_bid(&self, _from: u32) -> u32 {
        let mut best_tick = NONE;
        let mut best_px = i64::MIN;
        for (i, level) in
            self.active_levels.iter().enumerate()
        {
            if level.order_count == 0 {
                continue;
            }
            let head = self.orders.get(level.head);
            if head.side != Side::Buy as u8 {
                continue;
            }
            let px = head.price.0;
            if best_tick == NONE || px > best_px {
                best_tick = i as u32;
                best_px = px;
            }
        }
        best_tick
    }

    /// Find the tick of the lowest-priced non-empty ask level.
    /// See `scan_next_bid` for why this scans by price, not index.
    pub fn scan_next_ask(&self, _from: u32) -> u32 {
        let mut best_tick = NONE;
        let mut best_px = i64::MAX;
        for (i, level) in
            self.active_levels.iter().enumerate()
        {
            if level.order_count == 0 {
                continue;
            }
            let head = self.orders.get(level.head);
            if head.side != Side::Sell as u8 {
                continue;
            }
            let px = head.price.0;
            if best_tick == NONE || px < best_px {
                best_tick = i as u32;
                best_px = px;
            }
        }
        best_tick
    }
}
