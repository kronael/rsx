use crate::shard::RiskShard;
use rsx_messages::OrderRequest;
use uuid::Uuid;

/// LIQUIDATOR.md. Embedded liquidation engine.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidationStatus {
    Pending,
    Active,
    Done,
}

#[derive(Clone, Debug)]
pub struct LiquidationState {
    pub user_id: u32,
    pub symbol_id: u32,
    pub round: u32,
    pub status: LiquidationStatus,
    pub enqueued_at_ns: u64,
    pub last_order_ns: u64,
}

#[derive(Clone, Debug)]
pub struct LiquidationOrder {
    pub symbol_id: u32,
    pub user_id: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
}

#[derive(Clone, Debug)]
pub struct SocializedLoss {
    pub user_id: u32,
    pub symbol_id: u32,
    pub round: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
    pub timestamp_ns: u64,
}

pub struct LiquidationEngine {
    pub active: Vec<LiquidationState>,
    /// (user_id, symbol_id, retry_at_ns) for positions whose
    /// cascade ended in socialized loss. Re-liquidation is
    /// intended -- the position still exists and the exchange
    /// keeps drawing on it -- but at a human cadence, not the
    /// ~5.5 s cascade cadence.
    socialized_cooldown: Vec<(u32, u32, u64)>,
    socialized_cooldown_ns: u64,
    base_delay_ns: u64,
    base_slip_bps: i64,
    max_rounds: u32,
    max_slip_bps: i64,
}

impl LiquidationEngine {
    pub fn new(
        base_delay_ns: u64,
        base_slip_bps: i64,
        max_rounds: u32,
        max_slip_bps: i64,
        socialized_cooldown_ns: u64,
    ) -> Self {
        Self {
            active: Vec::new(),
            socialized_cooldown: Vec::new(),
            socialized_cooldown_ns,
            base_delay_ns,
            base_slip_bps,
            max_rounds,
            max_slip_bps,
        }
    }

    pub fn enqueue(&mut self, user_id: u32, symbol_id: u32, now_ns: u64) {
        self.socialized_cooldown
            .retain(|&(_, _, retry_at)| now_ns < retry_at);
        let cooling = self
            .socialized_cooldown
            .iter()
            .any(|&(u, s, _)| u == user_id && s == symbol_id);
        if cooling {
            return;
        }
        let already = self.active.iter().any(|s| {
            s.user_id == user_id && s.symbol_id == symbol_id && s.status != LiquidationStatus::Done
        });
        if already {
            return;
        }
        self.active.push(LiquidationState {
            user_id,
            symbol_id,
            round: 1,
            status: LiquidationStatus::Active,
            enqueued_at_ns: now_ns,
            last_order_ns: 0,
        });
    }

    pub fn maybe_process(
        &mut self,
        now_ns: u64,
        get_position_fn: &dyn Fn(u32, u32) -> i64,
        get_mark_fn: &dyn Fn(u32) -> i64,
    ) -> (Vec<LiquidationOrder>, Vec<SocializedLoss>) {
        // Nothing liquidating is the overwhelmingly common
        // case and this runs every busy-spin pass, so return
        // before allocating anything.
        if self.active.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let mut orders = Vec::with_capacity(4);
        let mut socialized = Vec::with_capacity(4);
        let mut cooled: Vec<(u32, u32)> = Vec::new();
        for state in &mut self.active {
            if state.status != LiquidationStatus::Active {
                continue;
            }
            // round == 1 means no order has been placed yet:
            // the first round always fires immediately. Gate on
            // the round counter (not `last_order_ns != 0`) to
            // avoid a sentinel collision when the first order is
            // placed at now_ns == 0 (a monotonic clock can start
            // at 0).
            let delay = (state.round as u64).saturating_mul(self.base_delay_ns);
            if state.round > 1 && now_ns < state.last_order_ns.saturating_add(delay) {
                continue;
            }
            let net_qty = get_position_fn(state.user_id, state.symbol_id);
            if net_qty == 0 {
                state.status = LiquidationStatus::Done;
                continue;
            }
            let mark = get_mark_fn(state.symbol_id);
            if mark == 0 {
                continue;
            }

            if state.round > self.max_rounds {
                let (side, price) = if net_qty > 0 {
                    (1u8, mark)
                } else {
                    (0u8, mark)
                };
                let qty = net_qty.abs();
                socialized.push(SocializedLoss {
                    user_id: state.user_id,
                    symbol_id: state.symbol_id,
                    round: state.round,
                    side,
                    price,
                    qty,
                    timestamp_ns: now_ns,
                });
                state.status = LiquidationStatus::Done;
                cooled.push((state.user_id, state.symbol_id));
                continue;
            }

            let slip = (state.round as i64)
                .checked_mul(state.round as i64)
                .and_then(|v| v.checked_mul(self.base_slip_bps))
                .map(|v| v.min(self.max_slip_bps))
                .unwrap_or(self.max_slip_bps);
            let (side, price) = if net_qty > 0 {
                let slip_price = mark
                    .checked_mul(10_000 - slip)
                    .map(|v| v / 10_000)
                    .unwrap_or(0);
                (1u8, slip_price)
            } else {
                let slip_price = mark
                    .checked_mul(10_000 + slip)
                    .map(|v| v / 10_000)
                    .unwrap_or(i64::MAX);
                (0u8, slip_price)
            };
            let qty = net_qty.abs();
            orders.push(LiquidationOrder {
                symbol_id: state.symbol_id,
                user_id: state.user_id,
                side,
                price,
                qty,
            });
            state.last_order_ns = now_ns;
            state.round += 1;
        }
        let retry_at = now_ns.saturating_add(self.socialized_cooldown_ns);
        for (user_id, symbol_id) in cooled {
            self.socialized_cooldown
                .push((user_id, symbol_id, retry_at));
        }
        (orders, socialized)
    }

    pub fn cancel_if_recovered(&mut self, user_id: u32, symbol_id: u32) {
        self.active
            .retain(|s| !(s.user_id == user_id && s.symbol_id == symbol_id));
    }

    pub fn remove_done(&mut self) {
        self.active.retain(|s| s.status != LiquidationStatus::Done);
    }

    pub fn is_in_liquidation(&self, user_id: u32, symbol_id: u32) -> bool {
        self.active.iter().any(|s| {
            s.user_id == user_id
                && s.symbol_id == symbol_id
                && s.status == LiquidationStatus::Active
        })
    }
}

impl RiskShard {
    /// Check if user needs liquidation after fill.
    /// Enqueues into liquidation engine if so.
    pub(crate) fn check_liquidation_for(&mut self, user_id: u32, now_ns: u64) {
        if !self.user_in_shard(user_id) {
            return;
        }
        // Use the cached fallback (mark, else index from BBO).
        // A silent mark feed (mark==0) must not make liquidation
        // inert when a valid index exists — same source the
        // pre-trade check uses, kept consistent here.
        if self.mark_dirty {
            self.rebuild_fallback();
        }
        let account = match self.accounts.get(&user_id) {
            Some(a) => a,
            None => return,
        };
        let frozen = self.frozen_for_user(user_id);
        let state = self.margin.calculate(
            account,
            self.iter_positions_for_user(user_id),
            &self.fallback_mark_prices,
            frozen,
        );
        let needs_liq = self.margin.needs_liquidation(&state);
        // Iterate the user's symbols directly instead of collecting a
        // Vec<u32> per fill: positions_by_user + positions are borrowed
        // immutably and liquidation mutably — disjoint fields, so no heap
        // allocation on this per-fill hot path.
        let Some(syms) = self.positions_by_user.get(&user_id) else {
            return;
        };
        for &sid in syms {
            let active = self
                .positions
                .get(&(user_id, sid))
                .is_some_and(|p| !p.is_empty());
            if !active {
                continue;
            }
            if needs_liq {
                self.liquidation.enqueue(user_id, sid, now_ns);
            } else {
                self.liquidation.cancel_if_recovered(user_id, sid);
            }
        }
    }

    /// Convert LiquidationOrder to OrderRequest. The order id is
    /// fresh per order; the price is rounded to the symbol's tick
    /// toward the aggressive side so it never gets less aggressive.
    pub(crate) fn liq_to_order(&self, liq: &LiquidationOrder, now_ns: u64) -> OrderRequest {
        let (order_id_hi, order_id_lo) = Uuid::now_v7().as_u64_pair();
        let tick = self
            .tick_size
            .get(liq.symbol_id as usize)
            .copied()
            .unwrap_or(0);
        OrderRequest {
            seq: 0,
            user_id: liq.user_id,
            symbol_id: liq.symbol_id,
            price: round_to_tick(liq.price, tick, liq.side),
            qty: liq.qty,
            order_id_hi,
            order_id_lo,
            timestamp_ns: now_ns,
            side: liq.side,
            tif: 1, // IOC
            reduce_only: true,
            post_only: false,
            is_liquidation: true,
            _pad: [0; 3],
        }
    }
}

/// A sell (side 1) floors to a multiple of `tick`, at least one tick;
/// a buy ceils. A zero `tick` (symbol not configured) leaves `price`.
fn round_to_tick(price: i64, tick: i64, side: u8) -> i64 {
    if tick <= 0 {
        return price;
    }
    let floor = price - price % tick;
    if side == 1 {
        return floor.max(tick);
    }
    if floor == price {
        return price;
    }
    floor.checked_add(tick).unwrap_or(floor)
}
