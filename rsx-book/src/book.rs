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
    pub compression: CompressionMap,
    pub state: BookState,
    pub config: SymbolConfig,
    pub sequence: u64,
    pub event_buf: [Event; MAX_EVENTS],
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
            compression,
            state: BookState::Normal,
            config,
            sequence: 0,
            event_buf: [Event::default(); MAX_EVENTS],
            event_len: 0,
            user_states: Vec::with_capacity(256),
            user_map: FxHashMap::default(),
            user_free_list: Vec::new(),
            user_bump: 0,
            old_levels: None,
            old_compression: None,
            bid_frontier: mid_price,
            ask_frontier: mid_price,
        }
    }

    #[inline]
    pub fn emit(&mut self, event: Event) {
        self.event_buf[self.event_len as usize] = event;
        self.event_len += 1;
    }

    pub fn events(&self) -> &[Event] {
        &self.event_buf[..self.event_len as usize]
    }

    pub fn is_migrating(&self) -> bool {
        self.state == BookState::Migrating
    }

    /// Insert a resting order into the book.
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
        slot.next = NONE;
        slot.prev = NONE;
        self.sequence += 1;
        slot.sequence = self.sequence as u16;

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

        // Update BBA
        match side {
            Side::Buy => {
                if self.best_bid_tick == NONE
                    || tick > self.best_bid_tick
                {
                    self.best_bid_tick = tick;
                }
            }
            Side::Sell => {
                if self.best_ask_tick == NONE
                    || tick < self.best_ask_tick
                {
                    self.best_ask_tick = tick;
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

    /// Cancel an order by slab handle.
    pub fn cancel_order(&mut self, handle: u32) -> bool {
        let slot = self.orders.get(handle);
        if !slot.is_active() {
            return false;
        }
        let tick = slot.tick_index;
        let side = slot.side;
        let qty = slot.remaining_qty.0;
        let prev = slot.prev;
        let next = slot.next;
        let user_id = slot.user_id;

        // Unlink
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
        level.total_qty -= qty;
        level.order_count -= 1;

        // Update BBA if needed
        if level.order_count == 0 {
            if side == Side::Buy as u8
                && tick == self.best_bid_tick
            {
                self.best_bid_tick =
                    self.scan_next_bid(tick);
            } else if side == Side::Sell as u8
                && tick == self.best_ask_tick
            {
                self.best_ask_tick =
                    self.scan_next_ask(tick);
            }
        }

        self.orders.get_mut(handle).set_active(false);
        self.orders.free(handle);

        // Decrement user order count
        if let Some(&uidx) =
            self.user_map.get(&user_id)
        {
            self.user_states[uidx as usize]
                .order_count -= 1;
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
    ) -> u32 {
        let qty = self.orders.get(handle).remaining_qty.0;
        self.cancel_order(handle);
        self.insert_resting(
            new_price, qty, side, tif, user_id,
            reduce_only, timestamp_ns,
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
        self.active_levels[tick as usize].total_qty -=
            diff;
        true
    }

    pub fn scan_next_bid(&self, from: u32) -> u32 {
        if from == 0 || from == NONE {
            return NONE;
        }
        let mut i = from - 1;
        loop {
            if self.active_levels[i as usize]
                .order_count
                > 0
            {
                return i;
            }
            if i == 0 {
                return NONE;
            }
            i -= 1;
        }
    }

    pub fn scan_next_ask(&self, from: u32) -> u32 {
        let max =
            self.active_levels.len() as u32 - 1;
        if from >= max || from == NONE {
            return NONE;
        }
        let mut i = from + 1;
        while i <= max {
            if self.active_levels[i as usize]
                .order_count
                > 0
            {
                return i;
            }
            i += 1;
        }
        NONE
    }
}
