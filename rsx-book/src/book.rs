use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::NONE;
use rustc_hash::FxHashMap;

use crate::compression::CompressionMap;
use crate::event::Event;
use crate::event::MAX_EVENTS;
use crate::level::PriceLevel;
use crate::occupancy::Occupancy;
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
    /// Per-side occupancy bitmaps over the compression slots: bit set =
    /// that level holds ≥1 resting order of that side. `scan_next_*`
    /// finds the next-best level by bitmap find-next-set instead of a
    /// linear O(slots) pass. Kept in perfect sync with
    /// `active_levels[t].order_count`: set on 0->non-empty, cleared on
    /// non-empty->0. Keyed by ORDER side (a buy sets `bid_occ`), which
    /// is what `scan_next_*` needs (a sell can rest below mid, i.e. in a
    /// bid-region index — the sawtooth is handled by `price_asc`).
    pub bid_occ: Occupancy,
    pub ask_occ: Occupancy,
    /// Index sub-ranges ordered by ascending price (see
    /// `build_price_asc`). Encodes the compression sawtooth so
    /// `scan_next_*` can walk occupied levels in true price order.
    /// Recomputed only on `new`/recenter.
    pub price_asc: Vec<(u32, u32)>,
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
    pub fn new(config: SymbolConfig, capacity: u32, mid_price: i64) -> Self {
        let compression = CompressionMap::new(mid_price, config.tick_size);
        let total = compression.total_slots() as usize;
        let active_levels = vec![PriceLevel::default(); total];
        let staging_levels = vec![PriceLevel::default(); total];
        let bid_occ = Occupancy::new(total as u32);
        let ask_occ = Occupancy::new(total as u32);
        let price_asc = build_price_asc(&compression);
        Self {
            active_levels,
            staging_levels,
            orders: Slab::new(capacity),
            best_bid_tick: NONE,
            best_ask_tick: NONE,
            best_bid_px: 0,
            best_ask_px: 0,
            compression,
            bid_occ,
            ask_occ,
            price_asc,
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
        let tick = self.compression.price_to_index(price);
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
        let level = &mut self.active_levels[tick as usize];
        let was_empty = level.order_count == 0;
        if was_empty {
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

        // Per-side occupancy: a compressed slot can hold BOTH sides, so
        // set the bit when THIS side's count goes 0 -> 1 — not on the
        // aggregate empty -> non-empty transition (which missed the 2nd
        // side and left occupancy stale: BOOK-STALE-OCC-ME-CRASH).
        let side_was_empty = match side {
            Side::Buy => {
                let e = level.bid_count == 0;
                level.bid_count += 1;
                e
            }
            Side::Sell => {
                let e = level.ask_count == 0;
                level.ask_count += 1;
                e
            }
        };
        if side_was_empty {
            match side {
                Side::Buy => self.bid_occ.set(tick),
                Side::Sell => self.ask_occ.set(tick),
            }
        }

        // Update BBA by RAW PRICE (compression index is a sawtooth,
        // not a price proxy — see best_bid_px doc).
        match side {
            Side::Buy => {
                if self.best_bid_tick == NONE || price > self.best_bid_px {
                    self.best_bid_tick = tick;
                    self.best_bid_px = price;
                }
            }
            Side::Sell => {
                if self.best_ask_tick == NONE || price < self.best_ask_px {
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
        self.user_states[uidx as usize].order_count += 1;

        handle
    }

    /// Unlink an order from its level's doubly-linked
    /// list. Adjusts head/tail, total_qty, order_count.
    /// Does not free the slab slot.
    pub fn unlink_order(&mut self, handle: u32) {
        let slot = self.orders.get(handle);
        let tick = slot.tick_index;
        let side = slot.side;
        let qty = slot.remaining_qty.0;
        let prev = slot.prev;
        let next = slot.next;
        let level = &mut self.active_levels[tick as usize];
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
        level.total_qty = level.total_qty.saturating_sub(qty);
        level.order_count = level.order_count.saturating_sub(1);

        // Per-side occupancy: clear when THIS side's count reaches 0, even
        // if the slot still holds the other side (mixed compressed slot).
        // The old aggregate-empty check left a departed side's bit set
        // while the other side remained (BOOK-STALE-OCC-ME-CRASH).
        let side_now_empty = if side == Side::Buy as u8 {
            level.bid_count = level.bid_count.saturating_sub(1);
            level.bid_count == 0
        } else {
            level.ask_count = level.ask_count.saturating_sub(1);
            level.ask_count == 0
        };
        if side_now_empty {
            if side == Side::Buy as u8 {
                self.bid_occ.clear(tick);
            } else {
                self.ask_occ.clear(tick);
            }
        }
    }

    /// Cancel an order by slab handle.
    ///
    /// The `is_active` guard makes a repeated cancel of the SAME handle a
    /// harmless no-op, but it cannot detect a STALE handle whose slot was
    /// freed and then reallocated to a different order (the slab reuses
    /// indices). Such a handle would silently cancel the wrong order.
    /// Cross-crate invariant rsx-matching must uphold: only call this with
    /// a handle whose identity you have verified, or go through
    /// `cancel_order_checked`, which re-checks `(user_id, order_id)`
    /// against the slot before cancelling. (`rsx-matching`'s user-cancel
    /// path already does this drift check inline; the WAL-replay path
    /// trusts its own `order_index`.)
    pub fn cancel_order(&mut self, handle: u32) -> bool {
        let slot = self.orders.get(handle);
        if !slot.is_active() {
            return false;
        }
        let tick = slot.tick_index;
        let side = slot.side;
        let user_id = slot.user_id;

        self.unlink_order(handle);

        // BBA can change even when the slot is NOT fully empty: a
        // compressed slot may lose its best-priced order of `side` while
        // other prices / the opposite side survive
        // (BOOK-STALE-BBA-WRONGFUL-POSTONLY). Refresh the affected side
        // whenever the cancelled order sat in that side's best level.
        if side == Side::Buy as u8 {
            if tick == self.best_bid_tick {
                self.refresh_best_bid();
            }
        } else if tick == self.best_ask_tick {
            self.refresh_best_ask();
        }

        self.orders.get_mut(handle).set_active(false);
        self.orders.free(handle);

        // Decrement user order count
        if let Some(&uidx) = self.user_map.get(&user_id) {
            self.user_states[uidx as usize].order_count = self.user_states[uidx as usize]
                .order_count
                .saturating_sub(1);
        }

        true
    }

    /// Cancel an order only if the slab slot at `handle` still holds the
    /// expected `(user_id, order_id_hi, order_id_lo)`. Returns `false`
    /// (book untouched) if the slot is inactive or has been reused by a
    /// different order — the stale-handle guard `cancel_order` alone
    /// cannot provide. Prefer this at cross-crate boundaries where the
    /// handle came from an external index that the slab may have recycled.
    pub fn cancel_order_checked(
        &mut self,
        handle: u32,
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
    ) -> bool {
        if (handle as usize) >= self.orders.capacity() as usize {
            return false;
        }
        let slot = self.orders.get(handle);
        if !slot.is_active()
            || slot.user_id != user_id
            || slot.order_id_hi != order_id_hi
            || slot.order_id_lo != order_id_lo
        {
            return false;
        }
        self.cancel_order(handle)
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
            new_price,
            qty,
            side,
            tif,
            user_id,
            reduce_only,
            timestamp_ns,
            order_id_hi,
            order_id_lo,
        )
    }

    /// Reduce remaining qty in-place. Preserves time
    /// priority. If new_qty == 0, cancels the order.
    /// Returns true if order was active.
    pub fn modify_order_qty_down(&mut self, handle: u32, new_qty: i64) -> bool {
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
        self.orders.get_mut(handle).remaining_qty = Qty(new_qty);
        self.active_levels[tick as usize].total_qty = self.active_levels[tick as usize]
            .total_qty
            .saturating_sub(diff);
        true
    }

    /// Highest resting BUY price at `tick` and the aggregate qty at that
    /// price, or (0, 0) if the slot holds no buys. Zone-0 slots hold one
    /// price of one side (O(1) via head/total_qty); a compressed slot may
    /// pack several prices and the opposite side, so it is walked. This is
    /// the true best bid within the level — the FIFO head is NOT (it can
    /// be a lower-priced buy, or a sell): BOOK-STALE-BBA-WRONGFUL-POSTONLY.
    pub(crate) fn bid_top_at(&self, tick: u32) -> (i64, i64) {
        if tick == NONE {
            return (0, 0);
        }
        let lvl = &self.active_levels[tick as usize];
        if lvl.bid_count == 0 {
            return (0, 0);
        }
        if (tick as usize) < self.compression.zone_slots[0] as usize {
            // Zone 0 is 1:1 => single price, single side (buy here).
            debug_assert_eq!(self.orders.get(lvl.head).side, Side::Buy as u8);
            return (self.orders.get(lvl.head).price.0, lvl.total_qty);
        }
        let mut px = i64::MIN;
        let mut qty = 0i64;
        let mut cursor = lvl.head;
        while cursor != NONE {
            let o = self.orders.get(cursor);
            if o.side == Side::Buy as u8 {
                if o.price.0 > px {
                    px = o.price.0;
                    qty = o.remaining_qty.0;
                } else if o.price.0 == px {
                    qty += o.remaining_qty.0;
                }
            }
            cursor = o.next;
        }
        (px, qty)
    }

    /// Lowest resting SELL price at `tick` and the aggregate qty at that
    /// price, or (0, 0) if the slot holds no sells. Mirror of `bid_top_at`.
    pub(crate) fn ask_top_at(&self, tick: u32) -> (i64, i64) {
        if tick == NONE {
            return (0, 0);
        }
        let lvl = &self.active_levels[tick as usize];
        if lvl.ask_count == 0 {
            return (0, 0);
        }
        if (tick as usize) < self.compression.zone_slots[0] as usize {
            debug_assert_eq!(self.orders.get(lvl.head).side, Side::Sell as u8);
            return (self.orders.get(lvl.head).price.0, lvl.total_qty);
        }
        let mut px = i64::MAX;
        let mut qty = 0i64;
        let mut cursor = lvl.head;
        while cursor != NONE {
            let o = self.orders.get(cursor);
            if o.side == Side::Sell as u8 {
                if o.price.0 < px {
                    px = o.price.0;
                    qty = o.remaining_qty.0;
                } else if o.price.0 == px {
                    qty += o.remaining_qty.0;
                }
            }
            cursor = o.next;
        }
        (px, qty)
    }

    /// Recompute `best_bid_tick`/`best_bid_px` from occupancy. Correct
    /// regardless of zone: `scan_next_bid` finds the highest-priced
    /// bid-occupied slot and `bid_top_at` reads the true best buy inside
    /// it. Call after any change that could touch the best bid.
    pub(crate) fn refresh_best_bid(&mut self) {
        self.best_bid_tick = self.scan_next_bid(NONE);
        self.best_bid_px = self.bid_top_at(self.best_bid_tick).0;
    }

    pub(crate) fn refresh_best_ask(&mut self) {
        self.best_ask_tick = self.scan_next_ask(NONE);
        self.best_ask_px = self.ask_top_at(self.best_ask_tick).0;
    }

    /// (bid_px, bid_qty, ask_px, ask_qty) for a BBO event. Uses the
    /// maintained best prices; qty is the resting qty of that side AT the
    /// best price (a compressed slot can hold the other side / other
    /// prices, so `total_qty` alone is not it).
    pub fn current_bbo(&self) -> (i64, i64, i64, i64) {
        let (bid_px, bid_qty) = self.bid_top_at(self.best_bid_tick);
        let (ask_px, ask_qty) = self.ask_top_at(self.best_ask_tick);
        (bid_px, bid_qty, ask_px, ask_qty)
    }

    /// Find the tick of the highest-priced non-empty BUY level.
    ///
    /// The compression map is a sawtooth: tick index is NOT globally
    /// price-monotonic, so a single find over the whole bitmap is wrong.
    /// Instead walk `price_asc` in DESCENDING price order and, in each
    /// sub-range (a zone half where ascending index == ascending price),
    /// take the highest set bit in `bid_occ` — the first hit is the
    /// max-price buy. In the common near-BBO case the first non-empty
    /// range is zone 0, so this returns after touching a handful of
    /// summary words. `_from` unused; kept for call-site symmetry.
    pub fn scan_next_bid(&self, _from: u32) -> u32 {
        for &(lo, hi) in self.price_asc.iter().rev() {
            if let Some(b) = self.bid_occ.find_last_in(lo, hi) {
                return b;
            }
        }
        NONE
    }

    /// Find the tick of the lowest-priced non-empty SELL level. Mirror
    /// of `scan_next_bid`: walk `price_asc` in ASCENDING price order and
    /// take the lowest set bit in `ask_occ` per sub-range.
    pub fn scan_next_ask(&self, _from: u32) -> u32 {
        for &(lo, hi) in self.price_asc.iter() {
            if let Some(b) = self.ask_occ.find_first_in(lo, hi) {
                return b;
            }
        }
        NONE
    }

    /// Rebuild both occupancy bitmaps AND the per-side counts from the
    /// orders linked into `active_levels`. Used after a snapshot load
    /// replaces the level array wholesale (the normal insert/cancel/match
    /// paths maintain both incrementally). A compressed slot can hold both
    /// sides, so walk the whole level — never key off the FIFO head side.
    pub fn rebuild_occupancy(&mut self) {
        let n = self.active_levels.len() as u32;
        self.bid_occ = Occupancy::new(n);
        self.ask_occ = Occupancy::new(n);
        for i in 0..self.active_levels.len() {
            let mut bid_count = 0u32;
            let mut ask_count = 0u32;
            let mut cursor = self.active_levels[i].head;
            while cursor != NONE {
                let o = self.orders.get(cursor);
                if o.side == Side::Buy as u8 {
                    bid_count += 1;
                } else {
                    ask_count += 1;
                }
                cursor = o.next;
            }
            self.active_levels[i].bid_count = bid_count;
            self.active_levels[i].ask_count = ask_count;
            if bid_count > 0 {
                self.bid_occ.set(i as u32);
            }
            if ask_count > 0 {
                self.ask_occ.set(i as u32);
            }
        }
    }
}

/// Bid-side index sub-range `[lo, hi)` for compression zone `z`.
/// Zone 4 is the catch-all (ask at `base`, bid at `base+1`, per
/// `price_to_index`); zones 0-3 split their slots in half (bid lower,
/// ask upper). A degenerate zone (`half == 0`) maps all its prices to
/// `base`, so its bid and ask ranges both cover the whole zone.
fn bid_region(comp: &CompressionMap, z: usize) -> (u32, u32) {
    let base = comp.base_indices[z];
    let slots = comp.zone_slots[z];
    if z == 4 {
        (base + 1, base + slots)
    } else {
        let half = slots / 2;
        if half == 0 {
            (base, base + slots)
        } else {
            (base, base + half)
        }
    }
}

/// Ask-side index sub-range `[lo, hi)` for zone `z`. See `bid_region`.
fn ask_region(comp: &CompressionMap, z: usize) -> (u32, u32) {
    let base = comp.base_indices[z];
    let slots = comp.zone_slots[z];
    if z == 4 {
        (base, base + 1)
    } else {
        let half = slots / 2;
        if half == 0 {
            (base, base + slots)
        } else {
            (base + half, base + slots)
        }
    }
}

/// Build the price-ascending list of index sub-ranges. Each entry is a
/// zone half in which ascending index == ascending price; the list
/// orders those runs by price band so `scan_next_*` can find the true
/// best level despite the compression sawtooth. Bids run zone 4
/// (furthest below mid) up to zone 0; asks run zone 0 up to zone 4.
/// Recomputed only on construction / recenter — never on the hot path.
pub(crate) fn build_price_asc(comp: &CompressionMap) -> Vec<(u32, u32)> {
    let mut v = Vec::with_capacity(10);
    for z in (0..=4usize).rev() {
        let (lo, hi) = bid_region(comp, z);
        if lo < hi {
            v.push((lo, hi));
        }
    }
    for z in 0..=4usize {
        let (lo, hi) = ask_region(comp, z);
        if lo < hi {
            v.push((lo, hi));
        }
    }
    v
}
