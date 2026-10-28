use crate::persist::FillRecord;
use crate::persist::PersistEvent;
use crate::shard::RiskShard;
use crate::types::FillEvent;
use rsx_types::calculate_fee;
use std::collections::hash_map::Entry;
use tracing::warn;

/// RISK.md §2. All fields i64 fixed-point.
#[derive(Clone, Debug, Default)]
#[repr(C, align(64))]
pub struct Position {
    pub user_id: u32,
    pub symbol_id: u32,
    pub long_qty: i64,
    pub short_qty: i64,
    pub long_entry_cost: i64,
    pub short_entry_cost: i64,
    pub realized_pnl: i64,
    pub last_fill_seq: u64,
    pub version: u64,
}

impl Position {
    pub fn new(user_id: u32, symbol_id: u32) -> Self {
        Self {
            user_id,
            symbol_id,
            ..Default::default()
        }
    }

    /// RISK.md §1. side: 0=Buy, 1=Sell.
    pub fn apply_fill(&mut self, side: u8, price: i64, qty: i64, seq: u64) {
        if side == 0 {
            // Buy fill
            if self.short_qty > 0 {
                // Opposing: reduce short
                let close_qty = qty.min(self.short_qty);
                let close_cost =
                    (self.short_entry_cost as i128 * close_qty as i128 / self.short_qty as i128)
                        .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
                self.realized_pnl += (close_cost as i128 - price as i128 * close_qty as i128)
                    .clamp(i64::MIN as i128, i64::MAX as i128)
                    as i64;
                self.short_qty -= close_qty;
                self.short_entry_cost -= close_cost;
                let remaining = qty - close_qty;
                if remaining > 0 {
                    self.long_qty += remaining;
                    self.long_entry_cost += (price as i128 * remaining as i128)
                        .clamp(i64::MIN as i128, i64::MAX as i128)
                        as i64;
                }
            } else {
                // Accumulate long
                self.long_qty += qty;
                self.long_entry_cost +=
                    (price as i128 * qty as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            }
        } else {
            // Sell fill
            if self.long_qty > 0 {
                // Opposing: reduce long
                let close_qty = qty.min(self.long_qty);
                let close_cost =
                    (self.long_entry_cost as i128 * close_qty as i128 / self.long_qty as i128)
                        .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
                self.realized_pnl += (price as i128 * close_qty as i128 - close_cost as i128)
                    .clamp(i64::MIN as i128, i64::MAX as i128)
                    as i64;
                self.long_qty -= close_qty;
                self.long_entry_cost -= close_cost;
                let remaining = qty - close_qty;
                if remaining > 0 {
                    self.short_qty += remaining;
                    self.short_entry_cost += (price as i128 * remaining as i128)
                        .clamp(i64::MIN as i128, i64::MAX as i128)
                        as i64;
                }
            } else {
                // Accumulate short
                self.short_qty += qty;
                self.short_entry_cost +=
                    (price as i128 * qty as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            }
        }
        self.version += 1;
        self.last_fill_seq = seq;
    }

    pub fn net_qty(&self) -> i64 {
        self.long_qty - self.short_qty
    }

    /// RISK.md §3.
    pub fn notional(&self, mark_price: i64) -> Result<i64, &'static str> {
        let v = self.net_qty().abs() as i128 * mark_price as i128;
        i64::try_from(v).map_err(|_| "notional overflow")
    }

    pub fn avg_entry(&self) -> i64 {
        let nq = self.net_qty();
        if nq > 0 {
            if self.long_qty == 0 {
                return 0;
            }
            self.long_entry_cost / self.long_qty
        } else if nq < 0 {
            if self.short_qty == 0 {
                return 0;
            }
            self.short_entry_cost / self.short_qty
        } else {
            0
        }
    }

    /// RISK.md §3.
    pub fn unrealized_pnl(&self, mark_price: i64) -> Result<i64, &'static str> {
        let nq = self.net_qty();
        if nq == 0 {
            return Ok(0);
        }
        let v = nq as i128 * (mark_price - self.avg_entry()) as i128;
        i64::try_from(v).map_err(|_| "unrealized_pnl overflow")
    }

    pub fn is_empty(&self) -> bool {
        self.long_qty == 0 && self.short_qty == 0
    }
}

impl RiskShard {
    /// Stamp the fee rates `process_fill` charges into `fill`, so the
    /// gateway computes each side's fee from the same rates. The ME
    /// writes both rates as 0; the symbol config columns are `INT`.
    pub fn stamp_fill_fees(&self, fill: &mut rsx_messages::FillRecord) {
        let sid = fill.symbol_id as usize;
        if sid >= self.taker_fee_bps.len() {
            return;
        }
        fill.taker_fee_bps = self.taker_fee_bps[sid] as i32;
        fill.maker_fee_bps = self.maker_fee_bps[sid] as i32;
    }

    /// RISK.md §1. Process a fill from ME ring.
    ///
    /// Invariant #4 (Position = sum of fills): both taker and maker
    /// positions are updated via `Position::apply_fill` on every fill
    /// that passes the `seq <= tip` dedup gate. Invariant #5 (tips
    /// monotonic): the dedup gate guarantees `tips[sid]` only ever
    /// advances (set to `fill.seq` at the end on success).
    pub fn process_fill(&mut self, fill: &FillEvent) {
        let sid = fill.symbol_id as usize;
        if sid >= self.tips.len() {
            warn!("process_fill: symbol_id {} out of bounds", fill.symbol_id);
            return;
        }

        // Dedup: skip if seq <= tip for this symbol
        if fill.seq <= self.tips[sid] {
            return;
        }

        let taker_fee_val = calculate_fee(fill.qty, fill.price, self.taker_fee_bps[sid]);
        let maker_fee_val = calculate_fee(fill.qty, fill.price, self.maker_fee_bps[sid]);

        // Process taker if in shard
        if self.user_in_shard(fill.taker_user_id) {
            self.ensure_account(fill.taker_user_id);
            self.ensure_position(fill.taker_user_id, fill.symbol_id);
            // INVARIANT position-after-ensure / account-after-ensure:
            // `ensure_position`/`ensure_account` ran immediately above,
            // so a miss here is a programming error. Returning instead
            // would leave the position mutated with the tip unadvanced,
            // and replay would apply the same fill twice (#4).
            let pos = self
                .positions
                .get_mut(&(fill.taker_user_id, fill.symbol_id))
                .expect("taker position missing after ensure_position");
            pos.apply_fill(fill.taker_side, fill.price, fill.qty, fill.seq);
            let acct = self
                .accounts
                .get_mut(&fill.taker_user_id)
                .expect("taker account missing after ensure_account");
            acct.deduct_fee(taker_fee_val);
            self.update_exposure(fill.taker_user_id, fill.symbol_id);
        }

        // Process maker if in shard
        if self.user_in_shard(fill.maker_user_id) {
            self.ensure_account(fill.maker_user_id);
            self.ensure_position(fill.maker_user_id, fill.symbol_id);
            let maker_side = if fill.taker_side == 0 { 1u8 } else { 0u8 };
            let pos = self
                .positions
                .get_mut(&(fill.maker_user_id, fill.symbol_id))
                .expect("maker position missing after ensure_position");
            pos.apply_fill(maker_side, fill.price, fill.qty, fill.seq);
            let acct = self
                .accounts
                .get_mut(&fill.maker_user_id)
                .expect("maker account missing after ensure_account");
            acct.deduct_fee(maker_fee_val);
            self.update_exposure(fill.maker_user_id, fill.symbol_id);
        }

        self.tips[sid] = fill.seq;
        self.fills_processed += 1;

        // Check liquidation for both sides
        self.check_liquidation_for(fill.taker_user_id, fill.timestamp_ns);
        self.check_liquidation_for(fill.maker_user_id, fill.timestamp_ns);

        // Persist the fill row (taker's shard only) + updated positions + tip
        if self.user_in_shard(fill.taker_user_id) {
            self.push_persist(PersistEvent::Fill(FillRecord {
                symbol_id: fill.symbol_id,
                taker_user_id: fill.taker_user_id,
                maker_user_id: fill.maker_user_id,
                price: fill.price,
                qty: fill.qty,
                taker_fee: taker_fee_val,
                maker_fee: maker_fee_val,
                taker_side: fill.taker_side,
                seq: fill.seq,
                timestamp_ns: fill.timestamp_ns,
            }));
            if let Some(p) = self.positions.get(&(fill.taker_user_id, fill.symbol_id)) {
                self.push_persist(PersistEvent::Position(p.clone()));
            }
            if let Some(a) = self.accounts.get(&fill.taker_user_id) {
                self.push_persist(PersistEvent::Account(a.clone()));
            }
        }
        if self.user_in_shard(fill.maker_user_id) {
            if let Some(p) = self.positions.get(&(fill.maker_user_id, fill.symbol_id)) {
                self.push_persist(PersistEvent::Position(p.clone()));
            }
            if let Some(a) = self.accounts.get(&fill.maker_user_id) {
                self.push_persist(PersistEvent::Account(a.clone()));
            }
        }
        self.push_persist(PersistEvent::Tip {
            symbol_id: fill.symbol_id,
            seq: fill.seq,
        });
    }

    fn update_exposure(&mut self, user_id: u32, symbol_id: u32) {
        let pos = &self.positions[&(user_id, symbol_id)];
        if pos.is_empty() {
            self.exposure.remove_user(symbol_id as usize, user_id);
        } else {
            self.exposure.add_user(symbol_id as usize, user_id);
        }
    }

    fn ensure_position(&mut self, user_id: u32, symbol_id: u32) {
        match self.positions.entry((user_id, symbol_id)) {
            Entry::Occupied(_) => {
                // Already present; index already has it
                // (maintained at the same moment as the
                // primary, no drift possible).
            }
            Entry::Vacant(v) => {
                v.insert(Position::new(user_id, symbol_id));
                self.positions_by_user
                    .entry(user_id)
                    .or_default()
                    .insert(symbol_id);
            }
        }
    }

    pub(crate) fn iter_positions_for_user(
        &self,
        user_id: u32,
    ) -> impl Iterator<Item = &Position> + '_ {
        let positions = &self.positions;
        self.positions_by_user
            .get(&user_id)
            .into_iter()
            .flat_map(move |symbols| {
                symbols
                    .iter()
                    .filter_map(move |&symbol_id| positions.get(&(user_id, symbol_id)))
            })
    }
}
