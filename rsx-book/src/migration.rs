use rsx_types::Side;
use rsx_types::NONE;

use crate::book::build_price_asc;
use crate::book::BookState;
use crate::book::Orderbook;
use crate::compression::CompressionMap;
use crate::level::PriceLevel;
use crate::occupancy::Occupancy;

impl Orderbook {
    /// Check if recentering is needed (mid drifted
    /// beyond 50% of zone 0 width).
    pub fn should_recenter(&self, current_mid: i64) -> bool {
        let zone0_half = self.compression.thresholds[0];
        let drift = (current_mid - self.compression.mid_price).abs();
        drift > zone0_half / 2
    }

    /// Trigger recentering: swap arrays, compute new
    /// compression map.
    pub fn trigger_recenter(&mut self, new_mid: i64) {
        let new_compression = CompressionMap::new(new_mid, self.config.tick_size);
        let new_total = new_compression.total_slots() as usize;

        // Swap: staging becomes new active, old
        // active becomes old_levels for migration
        let mut new_levels = std::mem::take(&mut self.staging_levels);
        // Resize/clear staging for new size
        new_levels.clear();
        new_levels.resize(new_total, PriceLevel::default());

        let old_levels = std::mem::replace(&mut self.active_levels, new_levels);
        let old_compression = std::mem::replace(&mut self.compression, new_compression);

        // Pre-allocate staging for next recenter
        let staging_total = self.compression.total_slots() as usize;
        self.staging_levels = vec![PriceLevel::default(); staging_total];

        self.old_levels = Some(old_levels);
        self.old_compression = Some(old_compression);
        if let Some(ref old) = self.old_compression {
            self.old_min_price = old.min_price();
            self.old_max_price = old.max_price();
        }
        self.bid_frontier = new_mid;
        self.ask_frontier = new_mid;
        self.state = BookState::Migrating;

        // Fresh (empty) active array + new compression => reset the
        // occupancy bitmaps and price-ordered ranges to match. Bits are
        // re-set per order as `migrate_single_level` fills the new array.
        let new_total = self.active_levels.len() as u32;
        self.bid_occ = Occupancy::new(new_total);
        self.ask_occ = Occupancy::new(new_total);
        self.price_asc = build_price_asc(&self.compression);

        // Re-scan BBA in new array
        self.best_bid_tick = NONE;
        self.best_ask_tick = NONE;
        self.best_bid_px = 0;
        self.best_ask_px = 0;

        // The frontier starts AT new_mid and every later step (lazy
        // `advance_frontier_to`, eager full migration) moves it away from
        // new_mid *before* migrating, so a level resting exactly at
        // new_mid would never be migrated and its orders would be dropped
        // when `old_levels` is cleared (MIGRATE-SKIPS-NEW-MID-LEVEL). So
        // migrate that one level once, here. It's within the [new_mid,
        // new_mid] frontier afterwards, so no later step migrates it
        // again; a no-op if nothing rests at new_mid.
        self.migrate_price(new_mid);
    }

    /// Resolve a level, migrating lazily if needed.
    pub fn resolve_level(&mut self, price: i64) {
        if self.state == BookState::Migrating && !self.is_within_frontier(price) {
            self.advance_frontier_to(price);
        }
    }

    pub fn is_within_frontier(&self, price: i64) -> bool {
        price >= self.bid_frontier && price <= self.ask_frontier
    }

    fn advance_frontier_to(&mut self, price: i64) {
        if price < self.bid_frontier {
            // Expand bid frontier down
            while self.bid_frontier > price {
                self.bid_frontier = self.bid_frontier.saturating_sub(self.config.tick_size);
                self.migrate_price(self.bid_frontier);
            }
        } else if price > self.ask_frontier {
            // Expand ask frontier up
            while self.ask_frontier < price {
                self.ask_frontier += self.config.tick_size;
                self.migrate_price(self.ask_frontier);
            }
        }
    }

    fn migrate_price(&mut self, price: i64) {
        let old_comp = match &self.old_compression {
            Some(c) => c,
            None => return,
        };
        let old_idx = old_comp.price_to_index(price);
        self.migrate_single_level(old_idx);
    }

    pub fn migrate_single_level(&mut self, old_idx: u32) {
        let old_levels = match &mut self.old_levels {
            Some(l) => l,
            None => return,
        };
        if old_idx as usize >= old_levels.len() {
            return;
        }
        let old_level = old_levels[old_idx as usize];
        if old_level.order_count == 0 {
            return;
        }

        let mut cursor = old_level.head;
        while cursor != NONE {
            let next = self.orders.get(cursor).next;
            let price = self.orders.get(cursor).price.0;
            let qty = self.orders.get(cursor).remaining_qty.0;
            let side = self.orders.get(cursor).side;
            let new_idx = self.compression.price_to_index(price);

            // Unlink from old, insert into new
            let new_level = &mut self.active_levels[new_idx as usize];
            let was_empty = new_level.order_count == 0;
            if was_empty {
                new_level.head = cursor;
                new_level.tail = cursor;
                self.orders.get_mut(cursor).prev = NONE;
                self.orders.get_mut(cursor).next = NONE;
            } else {
                let old_tail = new_level.tail;
                self.orders.get_mut(old_tail).next = cursor;
                self.orders.get_mut(cursor).prev = old_tail;
                self.orders.get_mut(cursor).next = NONE;
                new_level.tail = cursor;
            }
            new_level.total_qty += qty;
            new_level.order_count += 1;
            // Per-side occupancy: a migrated destination slot can receive
            // BOTH sides, so key the bit on THIS side's count going 0 -> 1,
            // not on the aggregate empty -> non-empty (BOOK-STALE-OCC).
            let side_was_empty = if side == Side::Buy as u8 {
                let e = new_level.bid_count == 0;
                new_level.bid_count += 1;
                e
            } else {
                let e = new_level.ask_count == 0;
                new_level.ask_count += 1;
                e
            };
            self.orders.get_mut(cursor).tick_index = new_idx;

            if side_was_empty {
                if side == Side::Buy as u8 {
                    self.bid_occ.set(new_idx);
                } else {
                    self.ask_occ.set(new_idx);
                }
            }

            // Update BBA by RAW PRICE (index is a sawtooth).
            if side == Side::Buy as u8 {
                if self.best_bid_tick == NONE || price > self.best_bid_px {
                    self.best_bid_tick = new_idx;
                    self.best_bid_px = price;
                }
            } else if side == Side::Sell as u8
                && (self.best_ask_tick == NONE || price < self.best_ask_px)
            {
                self.best_ask_tick = new_idx;
                self.best_ask_px = price;
            }

            cursor = next;
        }

        // Clear old level
        if let Some(l) = &mut self.old_levels {
            l[old_idx as usize] = PriceLevel::default();
        }
    }

    /// Batch migration during idle cycles.
    pub fn migrate_batch(&mut self, batch_size: u32) {
        if self.state != BookState::Migrating {
            return;
        }
        let tick = self.config.tick_size;
        let mut migrated = 0u32;

        while migrated < batch_size {
            // Expand bid frontier down
            if self.bid_frontier > 0 {
                self.bid_frontier = self.bid_frontier.saturating_sub(tick);
                self.migrate_price(self.bid_frontier);
                migrated += 1;
            }

            // Expand ask frontier up
            self.ask_frontier += tick;
            self.migrate_price(self.ask_frontier);
            migrated += 1;

            if self.bid_frontier <= self.old_min_price && self.ask_frontier >= self.old_max_price {
                self.complete_migration();
                break;
            }
        }
    }

    fn complete_migration(&mut self) {
        self.staging_levels = self.old_levels.take().unwrap_or_default();
        // Clear staging for reuse
        for l in &mut self.staging_levels {
            *l = PriceLevel::default();
        }
        self.old_compression = None;
        self.state = BookState::Normal;
    }
}
