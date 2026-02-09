use rsx_types::NONE;
use rsx_types::Side;

use crate::book::BookState;
use crate::book::Orderbook;
use crate::compression::CompressionMap;
use crate::level::PriceLevel;

impl Orderbook {
    /// Check if recentering is needed (mid drifted
    /// beyond 50% of zone 0 width).
    pub fn should_recenter(
        &self,
        current_mid: i64,
    ) -> bool {
        let zone0_half =
            self.compression.thresholds[0];
        let drift = (current_mid
            - self.compression.mid_price)
            .abs();
        drift > zone0_half / 2
    }

    /// Trigger recentering: swap arrays, compute new
    /// compression map.
    pub fn trigger_recenter(
        &mut self,
        new_mid: i64,
    ) {
        let new_compression = CompressionMap::new(
            new_mid,
            self.config.tick_size,
        );
        let new_total =
            new_compression.total_slots() as usize;

        // Swap: staging becomes new active, old
        // active becomes old_levels for migration
        let mut new_levels =
            std::mem::take(&mut self.staging_levels);
        // Resize/clear staging for new size
        new_levels.clear();
        new_levels.resize(
            new_total,
            PriceLevel::default(),
        );

        let old_levels =
            std::mem::replace(
                &mut self.active_levels,
                new_levels,
            );
        let old_compression =
            std::mem::replace(
                &mut self.compression,
                new_compression,
            );

        // Pre-allocate staging for next recenter
        let staging_total =
            self.compression.total_slots() as usize;
        self.staging_levels =
            vec![PriceLevel::default(); staging_total];

        self.old_levels = Some(old_levels);
        self.old_compression = Some(old_compression);
        self.bid_frontier = new_mid;
        self.ask_frontier = new_mid;
        self.state = BookState::Migrating;

        // Re-scan BBA in new array
        self.best_bid_tick = NONE;
        self.best_ask_tick = NONE;
    }

    /// Resolve a level, migrating lazily if needed.
    pub fn resolve_level(
        &mut self,
        price: i64,
    ) {
        if self.state == BookState::Migrating
            && !self.is_within_frontier(price)
        {
            self.advance_frontier_to(price);
        }
    }

    pub fn is_within_frontier(
        &self,
        price: i64,
    ) -> bool {
        price >= self.bid_frontier
            && price <= self.ask_frontier
    }

    fn advance_frontier_to(
        &mut self,
        price: i64,
    ) {
        if price < self.bid_frontier {
            // Expand bid frontier down
            while self.bid_frontier > price {
                self.bid_frontier -=
                    self.config.tick_size;
                self.migrate_price(
                    self.bid_frontier,
                );
            }
        } else if price > self.ask_frontier {
            // Expand ask frontier up
            while self.ask_frontier < price {
                self.ask_frontier +=
                    self.config.tick_size;
                self.migrate_price(
                    self.ask_frontier,
                );
            }
        }
    }

    fn migrate_price(&mut self, price: i64) {
        let old_comp = match &self.old_compression {
            Some(c) => c,
            None => return,
        };
        let old_idx =
            old_comp.price_to_index(price);
        self.migrate_single_level(old_idx);
    }

    pub fn migrate_single_level(
        &mut self,
        old_idx: u32,
    ) {
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
            let next =
                self.orders.get(cursor).next;
            let price =
                self.orders.get(cursor).price;
            let qty =
                self.orders.get(cursor)
                    .remaining_qty;
            let side =
                self.orders.get(cursor).side;
            let new_idx = self
                .compression
                .price_to_index(price);

            // Unlink from old, insert into new
            let new_level = &mut self.active_levels
                [new_idx as usize];
            if new_level.order_count > 0 {
                let old_tail = new_level.tail;
                self.orders.get_mut(old_tail).next =
                    cursor;
                self.orders.get_mut(cursor).prev =
                    old_tail;
                self.orders.get_mut(cursor).next =
                    NONE;
                new_level.tail = cursor;
            } else {
                new_level.head = cursor;
                new_level.tail = cursor;
                self.orders.get_mut(cursor).prev =
                    NONE;
                self.orders.get_mut(cursor).next =
                    NONE;
            }
            new_level.total_qty += qty;
            new_level.order_count += 1;
            self.orders.get_mut(cursor).tick_index =
                new_idx;

            // Update BBA
            if side == Side::Buy as u8 {
                if self.best_bid_tick == NONE
                    || new_idx > self.best_bid_tick
                {
                    self.best_bid_tick = new_idx;
                }
            } else if self.best_ask_tick == NONE
                || new_idx < self.best_ask_tick
            {
                self.best_ask_tick = new_idx;
            }

            cursor = next;
        }

        // Clear old level
        if let Some(l) = &mut self.old_levels {
            l[old_idx as usize] =
                PriceLevel::default();
        }
    }

    /// Batch migration during idle cycles.
    pub fn migrate_batch(
        &mut self,
        batch_size: u32,
    ) {
        if self.state != BookState::Migrating {
            return;
        }
        let tick = self.config.tick_size;
        let mut migrated = 0u32;

        while migrated < batch_size {
            // Expand bid frontier down
            if self.bid_frontier > 0 {
                self.bid_frontier -= tick;
                self.migrate_price(
                    self.bid_frontier,
                );
                migrated += 1;
            }

            // Expand ask frontier up
            self.ask_frontier += tick;
            self.migrate_price(self.ask_frontier);
            migrated += 1;

            // Check if migration complete
            if self.is_migration_complete() {
                self.complete_migration();
                break;
            }
        }
    }

    fn is_migration_complete(&self) -> bool {
        let old_levels = match &self.old_levels {
            Some(l) => l,
            None => return true,
        };
        old_levels.iter().all(|l| l.order_count == 0)
    }

    fn complete_migration(&mut self) {
        self.staging_levels =
            self.old_levels.take().unwrap_or_default();
        // Clear staging for reuse
        for l in &mut self.staging_levels {
            *l = PriceLevel::default();
        }
        self.old_compression = None;
        self.state = BookState::Normal;
    }
}
