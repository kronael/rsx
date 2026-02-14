/// Pre-computed zone boundaries for price-to-index mapping.
/// Recomputed once per recenter, not per order.
pub struct CompressionMap {
    pub mid_price: i64,
    /// Absolute distance thresholds for zones 0-3.
    /// Zone 4 is everything beyond thresholds[3].
    pub thresholds: [i64; 4],
    /// Ticks-per-slot for each zone.
    pub compressions: [u32; 5],
    /// First array index for each zone.
    pub base_indices: [u32; 5],
    /// Number of slots per zone (both sides combined).
    pub zone_slots: [u32; 5],
}

impl CompressionMap {
    pub fn new(mid_price: i64, tick_size: i64) -> Self {
        // Zone boundaries as distance in ticks from mid
        // Zone 0: 0-5% = mid * 0.05 / tick_size ticks
        // Zone 1: 5-15%
        // Zone 2: 15-30%
        // Zone 3: 30-50%
        // Zone 4: 50%+ (catch-all, 2 slots)
        let pct_5 = mid_price * 5 / (100 * tick_size);
        let pct_15 = mid_price * 15 / (100 * tick_size);
        let pct_30 = mid_price * 30 / (100 * tick_size);
        let pct_50 = mid_price * 50 / (100 * tick_size);

        let thresholds = [pct_5, pct_15, pct_30, pct_50];
        let compressions: [u32; 5] = [1, 10, 100, 1000, 1];

        // Slots per zone (both sides)
        let z0 = (pct_5 * 2) as u32;
        let z1 = (((pct_15 - pct_5) * 2) / 10) as u32;
        let z2 = (((pct_30 - pct_15) * 2) / 100) as u32;
        let z3 = (((pct_50 - pct_30) * 2) / 1000) as u32;
        let z4 = 2u32; // one per side

        let zone_slots = [z0, z1, z2, z3, z4];
        let base_indices = [
            0,
            z0,
            z0 + z1,
            z0 + z1 + z2,
            z0 + z1 + z2 + z3,
        ];

        Self {
            mid_price,
            thresholds,
            compressions,
            base_indices,
            zone_slots,
        }
    }

    /// Bisection: 2-3 comparisons, no loops.
    #[inline(always)]
    pub fn price_to_index(
        &self,
        price: i64,
    ) -> u32 {
        let tick_dist = price - self.mid_price;
        let distance = tick_dist.unsigned_abs() as i64;
        // ask=0 (price >= mid), bid=1 (price < mid)
        let side: u32 =
            if tick_dist >= 0 { 0 } else { 1 };

        let zone = if distance < self.thresholds[1] {
            if distance < self.thresholds[0] {
                0
            } else {
                1
            }
        } else if distance < self.thresholds[2] {
            2
        } else if distance < self.thresholds[3] {
            3
        } else {
            4
        };

        if zone == 4 {
            return self.base_indices[4] + side;
        }

        let zone_start = if zone == 0 {
            0
        } else {
            self.thresholds[zone - 1]
        };
        let half = self.zone_slots[zone] / 2;
        if half == 0 {
            return self.base_indices[zone];
        }
        let local_offset = ((distance - zone_start)
            / self.compressions[zone] as i64)
            as u32;
        let local_offset = local_offset.min(half - 1);

        if side == 0 {
            // ask side: mid outward
            self.base_indices[zone] + half + local_offset
        } else {
            // bid side: mid outward (reversed)
            self.base_indices[zone] + half
                - 1
                - local_offset
        }
    }

    pub fn total_slots(&self) -> u32 {
        self.zone_slots.iter().sum()
    }

    pub fn min_price(&self) -> i64 {
        self.mid_price
            - self.thresholds[3]
    }

    pub fn max_price(&self) -> i64 {
        self.mid_price
            + self.thresholds[3]
    }
}
