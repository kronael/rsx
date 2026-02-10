use crate::account::Account;
use crate::config::ShardConfig;
use crate::funding;
use crate::funding::FundingConfig;
use crate::liquidation::LiquidationEngine;
use crate::liquidation::LiquidationOrder;
use crate::margin::ExposureIndex;
use crate::margin::PortfolioMargin;
use crate::persist::FundingPaymentRecord;
use crate::persist::PersistEvent;
use crate::persist::PersistFill;
use crate::position::Position;
use crate::price::IndexPrice;
use crate::replay::ColdStartState;
use crate::rings::OrderResponse;
use crate::rings::ShardRings;
use crate::risk_utils::calculate_fee;
use crate::types::BboUpdate;
use crate::types::FillEvent;
use crate::types::OrderRequest;
use crate::types::RejectReason;
use rtrb::Producer;
use rustc_hash::FxHashMap;
use tracing::info;
use tracing::warn;

pub struct RiskShard {
    shard_id: u32,
    shard_count: u32,
    max_symbols: usize,

    pub accounts: FxHashMap<u32, Account>,
    pub positions: FxHashMap<(u32, u32), Position>,
    margin: PortfolioMargin,
    pub index_prices: Vec<IndexPrice>,
    pub mark_prices: Vec<i64>,
    exposure: ExposureIndex,
    pub tips: Vec<u64>,
    funding_config: FundingConfig,
    pub last_funding_id: u64,
    taker_fee_bps: Vec<i64>,
    maker_fee_bps: Vec<i64>,
    stashed_bbo: Vec<Option<BboUpdate>>,

    pub config_versions: Vec<u64>,
    pub fills_processed: u64,
    pub orders_processed: u64,
    persist_producer: Option<Producer<PersistEvent>>,
    pub liquidation: LiquidationEngine,
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
            margin: PortfolioMargin {
                symbol_params: config.symbol_params,
            },
            index_prices: vec![
                IndexPrice::default();
                max
            ],
            mark_prices: vec![0i64; max],
            exposure: ExposureIndex::new(max),
            tips: vec![0u64; max],
            funding_config: config.funding_config,
            last_funding_id: 0,
            taker_fee_bps: config.taker_fee_bps,
            maker_fee_bps: config.maker_fee_bps,
            stashed_bbo: vec![None; max],
            config_versions: vec![0u64; max],
            fills_processed: 0,
            orders_processed: 0,
            persist_producer: None,
            liquidation: LiquidationEngine::new(
                config.liquidation_config.base_delay_ns,
                config.liquidation_config.base_slip_bps
                    as i64,
                config.liquidation_config.max_rounds,
            ),
        }
    }

    pub fn set_persist_producer(
        &mut self,
        producer: Producer<PersistEvent>,
    ) {
        self.persist_producer = Some(producer);
    }

    pub fn load_state(&mut self, state: ColdStartState) {
        self.accounts = state.accounts;
        self.positions = state.positions;
        let len = self.tips.len().min(state.tips.len());
        self.tips[..len]
            .copy_from_slice(&state.tips[..len]);
    }

    fn push_persist(&mut self, event: PersistEvent) {
        if let Some(ref mut p) = self.persist_producer {
            if p.push(event).is_err() {
                warn!("persist ring full, dropping");
            }
        }
    }

    pub fn user_in_shard(&self, user_id: u32) -> bool {
        user_id % self.shard_count == self.shard_id
    }

    fn ensure_account(&mut self, user_id: u32) {
        self.accounts
            .entry(user_id)
            .or_insert_with(|| Account::new(user_id, 0));
    }

    fn ensure_position(
        &mut self,
        user_id: u32,
        symbol_id: u32,
    ) {
        self.positions
            .entry((user_id, symbol_id))
            .or_insert_with(|| {
                Position::new(user_id, symbol_id)
            });
    }

    fn positions_for_user(
        &self,
        user_id: u32,
    ) -> Vec<&Position> {
        self.positions
            .values()
            .filter(|p| p.user_id == user_id)
            .collect()
    }

    /// RISK.md §1. Process a fill from ME ring.
    pub fn process_fill(&mut self, fill: &FillEvent) {
        let sid = fill.symbol_id as usize;

        // Dedup: skip if seq <= tip for this symbol
        if fill.seq <= self.tips[sid] {
            return;
        }

        let mut taker_fee_val = 0i64;
        let mut maker_fee_val = 0i64;

        // Process taker if in shard
        if self.user_in_shard(fill.taker_user_id) {
            self.ensure_account(fill.taker_user_id);
            self.ensure_position(
                fill.taker_user_id,
                fill.symbol_id,
            );
            let pos = self
                .positions
                .get_mut(&(
                    fill.taker_user_id,
                    fill.symbol_id,
                ))
                .unwrap();
            pos.apply_fill(
                fill.taker_side,
                fill.price,
                fill.qty,
                fill.seq,
            );
            taker_fee_val = calculate_fee(
                fill.qty,
                fill.price,
                self.taker_fee_bps[sid],
            );
            self.accounts
                .get_mut(&fill.taker_user_id)
                .unwrap()
                .deduct_fee(taker_fee_val);
            self.update_exposure(
                fill.taker_user_id,
                fill.symbol_id,
            );
        }

        // Process maker if in shard
        if self.user_in_shard(fill.maker_user_id) {
            self.ensure_account(fill.maker_user_id);
            self.ensure_position(
                fill.maker_user_id,
                fill.symbol_id,
            );
            let maker_side = if fill.taker_side == 0 {
                1u8
            } else {
                0u8
            };
            let pos = self
                .positions
                .get_mut(&(
                    fill.maker_user_id,
                    fill.symbol_id,
                ))
                .unwrap();
            pos.apply_fill(
                maker_side,
                fill.price,
                fill.qty,
                fill.seq,
            );
            maker_fee_val = calculate_fee(
                fill.qty,
                fill.price,
                self.maker_fee_bps[sid],
            );
            self.accounts
                .get_mut(&fill.maker_user_id)
                .unwrap()
                .deduct_fee(maker_fee_val);
            self.update_exposure(
                fill.maker_user_id,
                fill.symbol_id,
            );
        }

        self.tips[sid] = fill.seq;
        self.fills_processed += 1;

        // Check liquidation for both sides
        self.check_liquidation_for(
            fill.taker_user_id,
            fill.timestamp_ns,
        );
        self.check_liquidation_for(
            fill.maker_user_id,
            fill.timestamp_ns,
        );

        // Persist fill + updated positions + tip
        self.push_persist(PersistEvent::Fill(PersistFill {
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
        if self.user_in_shard(fill.taker_user_id) {
            if let Some(p) = self.positions.get(
                &(fill.taker_user_id, fill.symbol_id),
            ) {
                self.push_persist(
                    PersistEvent::Position(p.clone()),
                );
            }
            if let Some(a) =
                self.accounts.get(&fill.taker_user_id)
            {
                self.push_persist(
                    PersistEvent::Account(a.clone()),
                );
            }
        }
        if self.user_in_shard(fill.maker_user_id) {
            if let Some(p) = self.positions.get(
                &(fill.maker_user_id, fill.symbol_id),
            ) {
                self.push_persist(
                    PersistEvent::Position(p.clone()),
                );
            }
            if let Some(a) =
                self.accounts.get(&fill.maker_user_id)
            {
                self.push_persist(
                    PersistEvent::Account(a.clone()),
                );
            }
        }
        self.push_persist(PersistEvent::Tip {
            symbol_id: fill.symbol_id,
            seq: fill.seq,
        });
    }

    fn update_exposure(
        &mut self,
        user_id: u32,
        symbol_id: u32,
    ) {
        let pos = &self.positions[&(user_id, symbol_id)];
        if pos.is_empty() {
            self.exposure
                .remove_user(symbol_id as usize, user_id);
        } else {
            self.exposure
                .add_user(symbol_id as usize, user_id);
        }
    }

    /// Check if user needs liquidation after fill.
    /// Enqueues into liquidation engine if so.
    fn check_liquidation_for(
        &mut self,
        user_id: u32,
        now_ns: u64,
    ) {
        if !self.user_in_shard(user_id) {
            return;
        }
        let account = match self.accounts.get(&user_id) {
            Some(a) => a,
            None => return,
        };
        let positions = self.positions_for_user(user_id);
        let state = self.margin.calculate(
            account,
            &positions,
            &self.mark_prices,
        );
        if self.margin.needs_liquidation(&state) {
            let syms: Vec<u32> = positions
                .iter()
                .filter(|p| !p.is_empty())
                .map(|p| p.symbol_id)
                .collect();
            for sid in syms {
                self.liquidation.enqueue(
                    user_id, sid, now_ns,
                );
            }
        }
    }

    /// Convert LiquidationOrder to OrderRequest.
    fn liq_to_order(
        &self,
        liq: &LiquidationOrder,
        now_ns: u64,
    ) -> OrderRequest {
        OrderRequest {
            seq: 0,
            user_id: liq.user_id,
            symbol_id: liq.symbol_id,
            price: liq.price,
            qty: liq.qty,
            order_id_hi: 0,
            order_id_lo: 0,
            timestamp_ns: now_ns,
            side: liq.side,
            tif: 1, // IOC
            reduce_only: true,
            is_liquidation: true,
            _pad: [0; 4],
        }
    }

    /// RISK.md §6. Pre-trade risk check.
    pub fn process_order(
        &mut self,
        order: &OrderRequest,
    ) -> OrderResponse {
        if !self.user_in_shard(order.user_id) {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::NotInShard,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
        }

        self.ensure_account(order.user_id);

        let account = &self.accounts[&order.user_id];
        let positions =
            self.positions_for_user(order.user_id);

        // Liquidation check
        let state = self.margin.calculate(
            account,
            &positions,
            &self.mark_prices,
        );
        if self.margin.needs_liquidation(&state)
            && !order.is_liquidation
        {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::UserInLiquidation,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
        }

        let sid = order.symbol_id as usize;
        match self.margin.check_order(
            account,
            &positions,
            order,
            &self.mark_prices,
            self.taker_fee_bps[sid],
        ) {
            Ok(margin_needed) => {
                self.accounts
                    .get_mut(&order.user_id)
                    .unwrap()
                    .freeze_margin(margin_needed);
                self.orders_processed += 1;
                let acct =
                    self.accounts[&order.user_id].clone();
                self.push_persist(
                    PersistEvent::Account(acct),
                );
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

    /// Release frozen margin when order completes.
    pub fn process_order_done(
        &mut self,
        event: &crate::types::OrderDoneEvent,
    ) {
        if !self.user_in_shard(event.user_id) {
            return;
        }
        if let Some(acct) =
            self.accounts.get_mut(&event.user_id)
        {
            acct.release_margin(event.frozen_amount);
            let snapshot = acct.clone();
            self.push_persist(
                PersistEvent::Account(snapshot),
            );
        }
    }

    /// RISK.md §4. Update index price from BBO.
    pub fn process_bbo(&mut self, bbo: &BboUpdate) {
        let sid = bbo.symbol_id as usize;
        self.index_prices[sid].update_from_bbo(bbo);
    }

    pub fn update_mark(
        &mut self,
        symbol_id: u32,
        price: i64,
    ) {
        self.mark_prices[symbol_id as usize] = price;
    }

    /// Track config version per symbol. Future:
    /// update symbol_params, fee rates from metadata.
    pub fn process_config_applied(
        &mut self,
        symbol_id: u32,
        config_version: u64,
    ) {
        let sid = symbol_id as usize;
        if sid < self.config_versions.len() {
            self.config_versions[sid] = config_version;
        }
    }

    pub fn stash_bbo(&mut self, bbo: BboUpdate) {
        let sid = bbo.symbol_id as usize;
        self.stashed_bbo[sid] = Some(bbo);
    }

    pub fn drain_stashed_bbos(&mut self) {
        for sid in 0..self.max_symbols {
            if let Some(bbo) = self.stashed_bbo[sid].take()
            {
                self.process_bbo(&bbo);
            }
        }
    }

    /// RISK.md §5. Settle funding if interval elapsed.
    pub fn maybe_settle_funding(
        &mut self,
        now_secs: u64,
    ) {
        if !funding::is_settlement_due(
            self.last_funding_id,
            now_secs,
            self.funding_config.interval_secs,
        ) {
            return;
        }

        let new_id = funding::interval_id(
            now_secs,
            self.funding_config.interval_secs,
        );

        for sid in 0..self.max_symbols {
            let mark = self.mark_prices[sid];
            let index = self.index_prices[sid].price;
            if mark == 0 || index == 0 {
                continue;
            }
            let rate = funding::calculate_rate(
                mark,
                index,
                &self.funding_config,
            );
            let users: Vec<u32> = self
                .exposure
                .users_for_symbol(sid)
                .to_vec();
            for user_id in users {
                let key =
                    (user_id, sid as u32);
                if let Some(pos) =
                    self.positions.get(&key)
                {
                    let payment =
                        funding::calculate_payment(
                            pos.net_qty(),
                            mark,
                            rate,
                        );
                    if let Some(acct) = self
                        .accounts
                        .get_mut(&user_id)
                    {
                        acct.deduct_fee(payment);
                    }
                    let acct_clone = self
                        .accounts
                        .get(&user_id)
                        .cloned();
                    self.push_persist(
                        PersistEvent::FundingPayment(
                            FundingPaymentRecord {
                                user_id,
                                symbol_id: sid as u32,
                                amount: payment,
                                rate,
                                settlement_ts: now_secs,
                            },
                        ),
                    );
                    if let Some(acct) = acct_clone {
                        self.push_persist(
                            PersistEvent::Account(acct),
                        );
                    }
                }
            }
        }

        self.last_funding_id = new_id;
    }

    /// One iteration of the main loop.
    /// Priority: fills > orders > mark > bbo > funding.
    pub fn run_once(
        &mut self,
        rings: &mut ShardRings,
        now_secs: u64,
    ) {
        // 1. Drain all fills (highest priority)
        for consumer in &mut rings.fill_consumers {
            while let Ok(fill) = consumer.pop() {
                self.process_fill(&fill);
            }
        }

        // 1b. Process pending liquidations
        let now_ns = now_secs * 1_000_000_000;
        let liq_orders = {
            let positions = &self.positions;
            let mark_prices = &self.mark_prices;
            self.liquidation.maybe_process(
                now_ns,
                &|uid, sid| {
                    positions
                        .get(&(uid, sid))
                        .map(|p| p.net_qty())
                        .unwrap_or(0)
                },
                &|sid| mark_prices[sid as usize],
            )
        };
        for liq in &liq_orders {
            let order = self.liq_to_order(liq, now_ns);
            let resp = self.process_order(&order);
            if matches!(
                resp,
                OrderResponse::Accepted { .. }
            ) {
                let _ = rings
                    .accepted_producer
                    .push(order);
                info!(
                    "liquidation order sent: \
                     user={} symbol={} side={} qty={}",
                    liq.user_id,
                    liq.symbol_id,
                    liq.side,
                    liq.qty,
                );
            }
        }
        self.liquidation.remove_done();

        // 2. Drain orders
        while let Ok(order) = rings.order_consumer.pop() {
            let resp = self.process_order(&order);
            if matches!(
                resp,
                OrderResponse::Accepted { .. }
            ) {
                let _ = rings
                    .accepted_producer
                    .push(order.clone());
            }
            let _ = rings.response_producer.push(resp);
        }

        // 3. Drain mark price updates
        while let Ok(mark) = rings.mark_consumer.pop() {
            self.update_mark(mark.symbol_id, mark.price);
        }

        // 4. Drain BBOs
        for consumer in &mut rings.bbo_consumers {
            while let Ok(bbo) = consumer.pop() {
                self.stash_bbo(bbo);
            }
        }
        self.drain_stashed_bbos();

        // 5. Funding settlement
        self.maybe_settle_funding(now_secs);
    }
}
