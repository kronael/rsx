use crate::account::Account;
use crate::persist::FrozenOrderRecord;
use crate::persist::PersistEvent;
use crate::position::Position;
use crate::rings::OrderResponse;
use crate::risk_utils::calculate_fee;
use crate::shard::order_key;
use crate::shard::RiskShard;
use crate::types::OrderRequest;
use crate::types::RejectReason;
use std::collections::hash_map::Entry;
use tracing::error;
use tracing::warn;

/// RISK.md §3. Rates in fixed-point bps.
#[derive(Clone, Debug)]
pub struct SymbolRiskParams {
    pub initial_margin_rate: i64,
    pub maintenance_margin_rate: i64,
    pub max_leverage: i64,
}

/// RISK.md §3.
#[derive(Clone, Debug, Default)]
pub struct MarginState {
    pub equity: i64,
    pub unrealized_pnl: i64,
    pub initial_margin: i64,
    pub maintenance_margin: i64,
    pub available_margin: i64,
    /// Sum of |position| * mark across the portfolio, before the
    /// order under test. The leverage cap is expressed against
    /// this, not against margin.
    pub gross_notional: i64,
}

pub struct PortfolioMargin {
    pub symbol_params: Vec<SymbolRiskParams>,
}

impl PortfolioMargin {
    /// RISK.md §3. Full portfolio margin calculation.
    /// `frozen` is the sum of margin reserved for the
    /// account's open orders (see `RiskShard::frozen_for_user`).
    pub fn calculate<'a>(
        &self,
        account: &Account,
        positions: impl Iterator<Item = &'a Position>,
        mark_prices: &[i64],
        frozen: i64,
    ) -> MarginState {
        let mut upnl = 0i64;
        let mut im = 0i64;
        let mut mm = 0i64;
        let mut gross = 0i64;
        for pos in positions {
            let sid = pos.symbol_id as usize;
            if sid >= self.symbol_params.len() || sid >= mark_prices.len() {
                continue;
            }
            let mark = mark_prices[sid];
            let pos_upnl = pos.unrealized_pnl(mark).unwrap_or_else(|_| {
                error!(
                    "unrealized_pnl overflow: \
                     user={} symbol={} mark={}",
                    pos.user_id, pos.symbol_id, mark
                );
                i64::MIN
            });
            upnl = upnl.saturating_add(pos_upnl);
            let notional = pos.notional(mark).unwrap_or_else(|_| {
                error!(
                    "notional overflow: \
                         user={} symbol={} mark={}",
                    pos.user_id, pos.symbol_id, mark
                );
                i64::MAX
            });
            gross = gross.saturating_add(notional);
            let params = &self.symbol_params[sid];
            let im_calc = notional as i128 * params.initial_margin_rate as i128 / 10_000;
            im = im.saturating_add(i64::try_from(im_calc).unwrap_or(i64::MAX));
            let mm_calc = notional as i128 * params.maintenance_margin_rate as i128 / 10_000;
            mm = mm.saturating_add(i64::try_from(mm_calc).unwrap_or(i64::MAX));
        }
        let equity = account.collateral.saturating_add(upnl);
        let available = equity.saturating_sub(im).saturating_sub(frozen);
        MarginState {
            equity,
            unrealized_pnl: upnl,
            initial_margin: im,
            maintenance_margin: mm,
            available_margin: available,
            gross_notional: gross,
        }
    }

    /// RISK.md §6. Pre-trade check.
    pub fn check_order<'a>(
        &self,
        account: &Account,
        positions: impl Iterator<Item = &'a Position>,
        order: &OrderRequest,
        mark_prices: &[i64],
        taker_fee_bps: i64,
        frozen: i64,
    ) -> Result<i64, RejectReason> {
        if order.is_liquidation {
            return Ok(0);
        }
        if order.reduce_only {
            return Ok(0);
        }
        let state = self.calculate(account, positions, mark_prices, frozen);
        let sid = order.symbol_id as usize;
        if sid >= self.symbol_params.len() {
            return Err(RejectReason::InsufficientMargin);
        }
        let params = &self.symbol_params[sid];
        let notional_128 = order.price as i128 * order.qty as i128;
        let order_notional = i64::try_from(notional_128).unwrap_or_else(|_| {
            error!(
                "order notional overflow: \
                     user={} price={} qty={}",
                order.user_id, order.price, order.qty
            );
            i64::MAX
        });
        // Leverage cap. The initial-margin rate implies a cap, but
        // max_leverage is set per symbol and shown to traders, so it
        // is enforced explicitly rather than left to that implication.
        // Non-positive equity cannot support any new exposure.
        if params.max_leverage > 0 {
            let gross_after = state.gross_notional.saturating_add(order_notional);
            let allowed = state.equity.saturating_mul(params.max_leverage).max(0);
            if state.equity <= 0 || gross_after > allowed {
                error!(
                    "leverage cap: user={} symbol={} gross_after={} \
                     equity={} max_leverage={}",
                    order.user_id, order.symbol_id, gross_after, state.equity, params.max_leverage
                );
                return Err(RejectReason::InsufficientMargin);
            }
        }
        let margin_needed = calculate_margin_needed(params, order.price, order.qty, taker_fee_bps);
        if state.available_margin < margin_needed {
            return Err(RejectReason::InsufficientMargin);
        }
        Ok(margin_needed)
    }

    /// RISK.md §7.
    pub fn needs_liquidation(&self, state: &MarginState) -> bool {
        state.equity < state.maintenance_margin
    }
}

/// RISK.md §6. Margin frozen for one resting order:
/// `notional * initial_margin_rate / 10_000 + taker fee`, saturating
/// at `i64::MAX`. The same amount the pre-trade gate reserves and
/// WAL replay reconstructs.
pub fn calculate_margin_needed(
    params: &SymbolRiskParams,
    price: i64,
    qty: i64,
    taker_fee_bps: i64,
) -> i64 {
    let order_notional = i64::try_from(price as i128 * qty as i128).unwrap_or(i64::MAX);
    let order_im_128 = order_notional as i128 * params.initial_margin_rate as i128 / 10_000;
    let order_im = i64::try_from(order_im_128).unwrap_or(i64::MAX);
    order_im.saturating_add(calculate_fee(qty, price, taker_fee_bps))
}

/// RISK.md §3. Exposure index.
pub struct ExposureIndex {
    exposure: Vec<Vec<u32>>,
}

impl ExposureIndex {
    pub fn new(max_symbols: usize) -> Self {
        Self {
            exposure: vec![Vec::new(); max_symbols],
        }
    }

    pub fn add_user(&mut self, symbol_idx: usize, user_id: u32) {
        let users = &mut self.exposure[symbol_idx];
        if !users.contains(&user_id) {
            users.push(user_id);
        }
    }

    pub fn remove_user(&mut self, symbol_idx: usize, user_id: u32) {
        let users = &mut self.exposure[symbol_idx];
        users.retain(|&u| u != user_id);
    }

    pub fn users_for_symbol(&self, symbol_idx: usize) -> &[u32] {
        &self.exposure[symbol_idx]
    }
}

impl RiskShard {
    /// RISK.md §6. Pre-trade risk check.
    pub fn process_order(&mut self, order: &OrderRequest) -> OrderResponse {
        if !self.user_in_shard(order.user_id) {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::NotInShard,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
        }

        // Solvency guard: non-positive price/qty yields a negative
        // notional → negative margin_needed → a freeze that *adds*
        // available margin. The gateway validates this on the WS
        // path, but risk is the solvency authority — reject here
        // regardless (cheap top-of-function branch, defense in depth).
        if order.price <= 0 || order.qty <= 0 {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::InsufficientMargin,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
        }

        self.ensure_account(order.user_id);

        // Rebuild fallback before borrowing account/positions.
        if self.mark_dirty {
            self.rebuild_fallback();
        }

        let account = &self.accounts[&order.user_id];
        let frozen = self.frozen_for_user(order.user_id);

        // Use cached fallback (index fills mark=0 gaps).
        let mark_prices = &self.fallback_mark_prices;
        let state = self.margin.calculate(
            account,
            self.iter_positions_for_user(order.user_id),
            mark_prices,
            frozen,
        );
        if self.margin.needs_liquidation(&state) && !order.is_liquidation {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::UserInLiquidation,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
        }

        let sid = order.symbol_id as usize;
        if sid >= self.taker_fee_bps.len() {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::InsufficientMargin,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
        }
        match self.margin.check_order(
            account,
            self.iter_positions_for_user(order.user_id),
            order,
            mark_prices,
            self.taker_fee_bps[sid],
            frozen,
        ) {
            Ok(margin_needed) => {
                self.freeze(
                    order.user_id,
                    order.order_id_hi,
                    order.order_id_lo,
                    margin_needed,
                );
                self.orders_processed += 1;
                // The durable FrozenInsert is NOT written here.
                // WAL OrderAccepted (ME-confirmed) is the sole
                // authority for durable freezes: persisting the
                // freeze before ME confirms would, on a crash
                // between PG write and ME accept, leave PG holding
                // a freeze with no confirming OrderAccepted/release
                // in the WAL -> permanent phantom margin hold. The
                // in-memory freeze above stays (pre-trade gate);
                // `confirm_freeze` writes the durable record when
                // ME's RECORD_ORDER_ACCEPTED comes back.
                OrderResponse::Accepted {
                    user_id: order.user_id,
                    margin_reserved: margin_needed,
                    order_id_hi: order.order_id_hi,
                    order_id_lo: order.order_id_lo,
                }
            }
            Err(reason) => OrderResponse::Rejected {
                user_id: order.user_id,
                reason,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            },
        }
    }

    /// ME confirmed the order (RECORD_ORDER_ACCEPTED). Write
    /// the durable freeze to PG now — this is the sole point a
    /// freeze becomes durable, so PG can never hold a freeze the
    /// WAL never confirmed. Amount is taken from the in-memory
    /// `frozen_orders` set by `process_order`; if absent (order
    /// not seen on this shard, or already released) nothing is
    /// persisted.
    pub fn confirm_freeze(
        &mut self,
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
        symbol_id: u32,
    ) {
        if !self.user_in_shard(user_id) {
            return;
        }
        let key = order_key(order_id_hi, order_id_lo);
        let amount = match self.frozen_orders.get(&key) {
            Some(&(owner, amount)) if owner == user_id => amount,
            _ => return,
        };
        self.push_persist(PersistEvent::FrozenInsert(FrozenOrderRecord {
            user_id,
            order_id_hi,
            order_id_lo,
            symbol_id,
            amount,
        }));
    }

    /// Release frozen margin when order completes.
    pub fn release_frozen_for_order(&mut self, user_id: u32, order_id_hi: u64, order_id_lo: u64) {
        if !self.user_in_shard(user_id) {
            return;
        }
        let key = order_key(order_id_hi, order_id_lo);
        let Entry::Occupied(entry) = self.frozen_orders.entry(key) else {
            return;
        };
        let (owner, _) = *entry.get();
        if owner != user_id {
            // Impossible-state signal: an order-lifecycle record
            // whose user_id differs from the freeze owner for the
            // same 128-bit order id. Keep the entry — removing it
            // would strand the owner's frozen_by_user index and
            // permanently over-reserve the owner's margin.
            error!(
                "release_frozen_for_order owner mismatch: \
                 order_id=({},{}) owner={} caller={}; freeze kept",
                order_id_hi, order_id_lo, owner, user_id
            );
            return;
        }
        entry.remove();
        if let Some(per_user) = self.frozen_by_user.get_mut(&owner) {
            per_user.remove(&key);
            if per_user.is_empty() {
                self.frozen_by_user.remove(&owner);
            }
        }
        self.push_persist(PersistEvent::FrozenRemove {
            user_id,
            order_id_hi,
            order_id_lo,
        });
    }

    /// Reconstruct a frozen order from WAL replay.
    /// Same margin logic as process_order but without
    /// rejection (order was already accepted).
    pub fn replay_freeze_order(
        &mut self,
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
        price: i64,
        qty: i64,
        symbol_id: u32,
    ) {
        self.ensure_account(user_id);
        let sid = symbol_id as usize;
        if sid >= self.taker_fee_bps.len() || sid >= self.margin.symbol_params.len() {
            warn!("replay_freeze_order: sid {} out of bounds", sid);
            return;
        }
        let margin_needed = calculate_margin_needed(
            &self.margin.symbol_params[sid],
            price,
            qty,
            self.taker_fee_bps[sid],
        );
        self.freeze(user_id, order_id_hi, order_id_lo, margin_needed);
    }

    fn freeze(&mut self, user_id: u32, order_id_hi: u64, order_id_lo: u64, amount: i64) {
        let key = order_key(order_id_hi, order_id_lo);
        self.frozen_orders.insert(key, (user_id, amount));
        self.frozen_by_user
            .entry(user_id)
            .or_default()
            .insert(key, amount);
    }

    /// Sum of margin reserved for the user's open orders.
    /// Derived from `frozen_orders` via the per-user index;
    /// O(this user's open orders), not O(all open orders).
    pub fn frozen_for_user(&self, user_id: u32) -> i64 {
        match self.frozen_by_user.get(&user_id) {
            Some(per_user) => {
                let mut total: i64 = 0;
                for &amount in per_user.values() {
                    total = total.saturating_add(amount);
                }
                total
            }
            None => 0,
        }
    }
}
