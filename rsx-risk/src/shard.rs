use crate::account::Account;
use crate::config::ShardConfig;
use crate::config::SymbolRiskConfig;
use crate::insurance::InsuranceFund;
use crate::liquidation::LiquidationEngine;
use crate::liquidation::LIQUIDATION_PLACED;
use crate::margin::ExposureIndex;
use crate::margin::Freeze;
use crate::margin::PortfolioMargin;
use crate::persist::PersistEvent;
use crate::position::Position;
use crate::price::IndexPrice;
use crate::replay::ColdStartState;
use crate::rings::MeCommand;
use crate::rings::OrderResponse;
use crate::rings::ShardRings;
use crate::types::BboUpdate;
use rsx_messages::CancelRequest;
use rtrb::Consumer;
use rtrb::Producer;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use tracing::info;
use tracing::warn;

pub struct RiskShard {
    pub(crate) shard_id: u32,
    pub(crate) shard_count: u32,
    pub(crate) max_symbols: usize,

    pub accounts: FxHashMap<u32, Account>,
    pub positions: FxHashMap<(u32, u32), Position>,
    /// Per-user index over `positions`. user_id → set of
    /// symbol_ids this user has a position in. Maintained on
    /// every `ensure_position` and rebuilt on `set_state`.
    /// Replaces O(all_positions) scans in `positions_for_user`
    /// with O(this_user's_positions).
    pub(crate) positions_by_user: FxHashMap<u32, rustc_hash::FxHashSet<u32>>,
    pub(crate) margin: PortfolioMargin,
    pub index_prices: Vec<IndexPrice>,
    pub mark_prices: Vec<i64>,
    pub(crate) exposure: ExposureIndex,
    pub tips: Vec<u64>,
    pub funding_interval_secs: Vec<u64>,
    pub funding_min_bps: Vec<i64>,
    pub funding_max_bps: Vec<i64>,
    /// Last settled funding interval per symbol; `None` until the
    /// symbol has settled one.
    pub funding_settled: Vec<Option<u64>>,
    pub(crate) funding_checked_secs: u64,
    pub(crate) funding_stalled: bool,
    pub(crate) taker_fee_bps: Vec<i64>,
    pub(crate) maker_fee_bps: Vec<i64>,
    pub(crate) tick_size: Vec<i64>,
    pub(crate) stashed_bbo: Vec<Option<BboUpdate>>,
    pub insurance_funds: FxHashMap<u32, InsuranceFund>,
    pub(crate) frozen_orders: FxHashMap<u128, Freeze>,
    /// Cancels for the orders of a user who entered liquidation,
    /// drained onto the ME ring by `tick`.
    pub(crate) pending_cancels: VecDeque<CancelRequest>,
    /// Per-user index over `frozen_orders`. user_id →
    /// {order_id → frozen_amount}. Maintained on every
    /// frozen_orders insert/remove and rebuilt on `set_state`.
    /// Replaces O(all_frozen) scans in `frozen_for_user` with
    /// O(this_user's_open_orders).
    pub(crate) frozen_by_user: FxHashMap<u32, FxHashMap<u128, i64>>,

    pub config_versions: Vec<u64>,
    pub fills_processed: u64,
    pub orders_processed: u64,
    pub backpressured: bool,
    /// Debounce for the persist-ring-full warn: true while
    /// `push_persist` is inside a stall episode.
    pub(crate) persist_stalled: bool,
    pub(crate) persist_producer: Option<Producer<PersistEvent>>,
    pub(crate) config_updates: Option<Consumer<SymbolRiskConfig>>,
    pub liquidation: LiquidationEngine,

    // Cached mark prices with index-price fallback for
    // symbols where mark=0. Recomputed only when prices
    // change (mark_dirty flag), not per order.
    pub(crate) fallback_mark_prices: Vec<i64>,
    pub(crate) mark_dirty: bool,
}

impl RiskShard {
    pub fn new(config: ShardConfig) -> Self {
        let max = config.max_symbols;
        Self {
            shard_id: config.shard_id,
            shard_count: config.shard_count,
            max_symbols: max,
            accounts: FxHashMap::default(),
            positions: FxHashMap::default(),
            positions_by_user: FxHashMap::default(),
            margin: PortfolioMargin {
                symbol_params: config.symbol_params,
            },
            index_prices: vec![IndexPrice::default(); max],
            mark_prices: vec![0i64; max],
            exposure: ExposureIndex::new(max),
            tips: vec![0u64; max],
            funding_interval_secs: vec![0; max],
            funding_min_bps: vec![0; max],
            funding_max_bps: vec![0; max],
            funding_settled: vec![None; max],
            funding_checked_secs: u64::MAX,
            funding_stalled: false,
            taker_fee_bps: config.taker_fee_bps,
            maker_fee_bps: config.maker_fee_bps,
            tick_size: vec![0; max],
            stashed_bbo: vec![None; max],
            insurance_funds: FxHashMap::default(),
            frozen_orders: FxHashMap::default(),
            pending_cancels: VecDeque::new(),
            frozen_by_user: FxHashMap::default(),
            config_versions: vec![0u64; max],
            fills_processed: 0,
            orders_processed: 0,
            backpressured: false,
            persist_stalled: false,
            persist_producer: None,
            config_updates: None,
            fallback_mark_prices: vec![0i64; max],
            mark_dirty: true,
            liquidation: LiquidationEngine::new(
                config.liquidation_config.base_delay_ns,
                config.liquidation_config.base_slip_bps as i64,
                config.liquidation_config.max_rounds,
                config.liquidation_config.max_slip_bps as i64,
                config.liquidation_config.socialized_cooldown_ns,
            ),
        }
    }

    pub fn set_persist_producer(&mut self, producer: Producer<PersistEvent>) {
        self.persist_producer = Some(producer);
    }

    pub fn set_config_consumer(&mut self, consumer: Consumer<SymbolRiskConfig>) {
        self.config_updates = Some(consumer);
    }

    pub fn set_state(&mut self, state: ColdStartState) {
        self.accounts = state.accounts;
        self.positions = state.positions;
        self.insurance_funds = state.insurance_funds;
        self.frozen_orders = state.frozen_orders;
        // Rebuild secondary indexes from the loaded primaries.
        // Without this, frozen_for_user / positions_for_user
        // would see an empty index after cold-start and silently
        // return zero on a populated state.
        self.positions_by_user.clear();
        for &(user_id, symbol_id) in self.positions.keys() {
            self.positions_by_user
                .entry(user_id)
                .or_default()
                .insert(symbol_id);
        }
        self.frozen_by_user.clear();
        for (&order_id, freeze) in self.frozen_orders.iter() {
            self.frozen_by_user
                .entry(freeze.user_id)
                .or_default()
                .insert(order_id, freeze.amount);
        }
        self.exposure = ExposureIndex::new(self.max_symbols);
        for (&(user_id, symbol_id), position) in self.positions.iter() {
            if !position.is_empty() && (symbol_id as usize) < self.max_symbols {
                self.exposure.add_user(symbol_id as usize, user_id);
            }
        }
        let funding_len = self.funding_settled.len().min(state.funding_settled.len());
        self.funding_settled[..funding_len].copy_from_slice(&state.funding_settled[..funding_len]);
        let len = self.tips.len().min(state.tips.len());
        self.tips[..len].copy_from_slice(&state.tips[..len]);
    }

    pub fn user_in_shard(&self, user_id: u32) -> bool {
        user_id % self.shard_count == self.shard_id
    }

    /// One iteration of the main loop.
    /// Priority: fills > orders > mark > bbo > funding.
    /// Periodic tick: runs the liquidation sweep, tip/funding
    /// settlement, and BBO drain. Order/fill/bbo/mark INGEST is
    /// done directly by the recv loop (same thread) via
    /// `process_order`/`process_fill`/`stash_bbo`/`update_mark`.
    /// This keeps only the work that is time-driven rather than
    /// message-driven, plus the egress-ring backpressure stall.
    pub fn tick(&mut self, rings: &mut ShardRings, now_secs: u64) {
        self.drain_config_updates();

        // Downstream backpressure: if the ME command
        // or response ring is full, liquidation orders would
        // have to be dropped to push. Stall (skip this tick) so
        // upstream (gateway/main-loop) sees the backpressure and
        // queues at the receiver side. Warn only on the
        // transition — this runs at busy-spin frequency on the
        // pinned core. Persist-ring pressure does not gate the
        // tick: `push_persist` stalls inline, so the liquidation
        // sweep, funding settlement and BBO scans keep running.
        if rings.me_producer.is_full() || rings.response_producer.is_full() {
            if !self.backpressured {
                warn!(
                    "downstream ring full, stalling \
                     (me_full={} response_full={})",
                    rings.me_producer.is_full(),
                    rings.response_producer.is_full(),
                );
                self.backpressured = true;
            }
            return;
        }
        self.backpressured = false;

        self.drain_pending_cancels(rings);

        // 1. Process pending liquidations. Price the
        // liquidation rounds off the fallback (mark, else
        // index) so a silent mark feed doesn't make the engine
        // inert — consistent with the pre-trade check.
        let now_ns = now_secs * 1_000_000_000;
        if self.mark_dirty {
            self.rebuild_fallback();
        }
        let (liq_orders, socialized_losses) = {
            let positions = &self.positions;
            let mark_prices = &self.fallback_mark_prices;
            self.liquidation.maybe_process(
                now_ns,
                &|uid, sid| positions.get(&(uid, sid)).map(|p| p.net_qty()).unwrap_or(0),
                &|sid| {
                    let idx = sid as usize;
                    if idx < mark_prices.len() {
                        mark_prices[idx]
                    } else {
                        0
                    }
                },
            )
        };
        for liq in &liq_orders {
            if rings.me_producer.is_full() || rings.response_producer.is_full() {
                warn!(
                    "egress ring full mid-liquidation; \
                     stalling and retrying next tick",
                );
                self.backpressured = true;
                return;
            }
            let order = self.liq_to_order(liq, now_ns);
            let resp = self.process_order(&order);
            if matches!(resp, OrderResponse::Accepted { .. }) {
                rings
                    .me_producer
                    .push(MeCommand::Order(order, now_ns))
                    .expect(
                        "INVARIANT: me_producer \
                         capacity checked above",
                    );
                rings
                    .response_producer
                    .push(OrderResponse::Liquidation {
                        user_id: liq.user_id,
                        symbol_id: liq.symbol_id,
                        status: LIQUIDATION_PLACED,
                        round: liq.round,
                        side: liq.side,
                        qty: liq.qty,
                        price: order.price,
                        slip_bps: liq.slip_bps,
                    })
                    .expect(
                        "INVARIANT: response_producer \
                         capacity checked above",
                    );
                info!(
                    "liquidation order sent: \
                     user={} symbol={} side={} qty={}",
                    liq.user_id, liq.symbol_id, liq.side, liq.qty,
                );
            }
        }

        // Process socialized losses (LIQUIDATOR.md §9)
        for loss in &socialized_losses {
            let Some(notice) = self.process_socialized_loss(loss, now_ns) else {
                continue;
            };
            if rings.response_producer.push(notice).is_err() {
                warn!(
                    "response ring full; liquidation notice dropped \
                     user={} symbol={}",
                    loss.user_id, loss.symbol_id,
                );
            }
        }

        self.liquidation.remove_done();

        // 2. Apply coalesced BBOs (recv loop stashed them;
        // drain runs the per-BBO margin scan anchored on this
        // tick's now_ns).
        self.drain_stashed_bbos(now_ns);

        // 3. Funding settlement
        self.maybe_settle_funding(now_secs);
    }
}

pub(crate) fn order_key(order_id_hi: u64, order_id_lo: u64) -> u128 {
    ((order_id_hi as u128) << 64) | order_id_lo as u128
}
