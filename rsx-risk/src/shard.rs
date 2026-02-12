use crate::account::Account;
use crate::config::ShardConfig;
use crate::funding;
use crate::funding::FundingConfig;
use crate::insurance::InsuranceFund;
use crate::liquidation::LiquidationEngine;
use crate::liquidation::LiquidationOrder;
use crate::margin::ExposureIndex;
use crate::margin::PortfolioMargin;
use crate::persist::FundingPaymentRecord;
use crate::persist::LiquidationEventRecord;
use crate::persist::PersistEvent;
use crate::persist::PersistFill;
use crate::position::Position;
use crate::price::IndexPrice;
use crate::replica::ReplicaState;
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
    pub insurance_funds: FxHashMap<u32, InsuranceFund>,
    frozen_orders: FxHashMap<u128, (u32, i64)>,

    pub config_versions: Vec<u64>,
    pub fills_processed: u64,
    pub orders_processed: u64,
    pub backpressured: bool,
    persist_producer: Option<Producer<PersistEvent>>,
    replica_tip_producer: Option<Producer<(u32, u64)>>,
    pub replica_state: Option<ReplicaState>,
    pub liquidation: LiquidationEngine,
}

impl RiskShard {
    pub fn new(config: ShardConfig) -> Self {
        let max = config.max_symbols;
        let replica_state =
            if config.replication_config.is_replica {
                Some(ReplicaState::new(max))
            } else {
                None
            };
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
            insurance_funds: FxHashMap::default(),
            frozen_orders: FxHashMap::default(),
            config_versions: vec![0u64; max],
            fills_processed: 0,
            orders_processed: 0,
            backpressured: false,
            persist_producer: None,
            replica_tip_producer: None,
            replica_state,
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
        self.insurance_funds = state.insurance_funds;
        // frozen_orders rebuilt from WAL replay
        let len = self.tips.len().min(state.tips.len());
        self.tips[..len]
            .copy_from_slice(&state.tips[..len]);
    }

    fn push_persist(&mut self, event: PersistEvent) {
        if let Some(ref mut p) = self.persist_producer {
            if p.push(event).is_err() {
                warn!("persist ring full, stalling");
                self.backpressured = true;
            }
        }
    }

    /// Per WAL.md: persist ring full -> stall hot path.
    pub fn is_backpressured(&self) -> bool {
        if self.backpressured {
            return true;
        }
        if let Some(ref p) = self.persist_producer {
            return p.slots() == 0;
        }
        false
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
            // SAFETY: ensure_position() guarantees key exists
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
            // SAFETY: ensure_account() guarantees key exists
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
            // SAFETY: ensure_position() guarantees key exists
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
            // SAFETY: ensure_account() guarantees key exists
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
            post_only: false,
            is_liquidation: true,
            _pad: [0; 3],
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

        // Liquidation check (fallback to index when mark missing)
        let mut fallback = None;
        for (sid, mark) in self.mark_prices.iter().enumerate() {
            if *mark == 0 {
                let idx = &self.index_prices[sid];
                if idx.valid && idx.price > 0 {
                    let mut tmp = self.mark_prices.clone();
                    for (i, v) in tmp.iter_mut().enumerate() {
                        if *v == 0 {
                            let idx = &self.index_prices[i];
                            if idx.valid && idx.price > 0 {
                                *v = idx.price;
                            }
                        }
                    }
                    fallback = Some(tmp);
                    break;
                }
            }
        }
        let mark_prices = fallback
            .as_deref()
            .unwrap_or(&self.mark_prices);
        let state = self.margin.calculate(
            account,
            &positions,
            mark_prices,
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
            mark_prices,
            self.taker_fee_bps[sid],
        ) {
            Ok(margin_needed) => {
                // SAFETY: ensure_account() guarantees key exists
                self.accounts
                    .get_mut(&order.user_id)
                    .unwrap()
                    .freeze_margin(margin_needed);
                self.frozen_orders.insert(
                    order_key(
                        order.order_id_hi,
                        order.order_id_lo,
                    ),
                    (order.user_id, margin_needed),
                );
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
    pub fn release_frozen_for_order(
        &mut self,
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
    ) {
        if !self.user_in_shard(user_id) {
            return;
        }
        let key = order_key(order_id_hi, order_id_lo);
        let entry = self.frozen_orders.remove(&key);
        let Some((owner, amount)) = entry else {
            return;
        };
        if owner != user_id {
            return;
        }
        if let Some(acct) = self.accounts.get_mut(&user_id) {
            acct.release_margin(amount);
            let snapshot = acct.clone();
            self.push_persist(PersistEvent::Account(snapshot));
        }
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
        let order_notional =
            (price as i128 * qty as i128) as i64;
        let params = &self.margin.symbol_params[sid];
        let order_im = (order_notional as i128
            * params.initial_margin_rate as i128
            / 10_000) as i64;
        let order_fee = crate::risk_utils::calculate_fee(
            qty,
            price,
            self.taker_fee_bps[sid],
        );
        let margin_needed = order_im + order_fee;
        self.accounts
            .get_mut(&user_id)
            .unwrap()
            .freeze_margin(margin_needed);
        self.frozen_orders.insert(
            order_key(order_id_hi, order_id_lo),
            (user_id, margin_needed),
        );
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
        if sid >= self.config_versions.len() {
            return;
        }
        if config_version < self.config_versions[sid] {
            return;
        }
        self.config_versions[sid] = config_version;
        self.reload_symbol_overrides(symbol_id);
    }

    fn reload_symbol_overrides(&mut self, symbol_id: u32) {
        let sid = symbol_id as usize;
        let taker_key =
            format!("RSX_SYMBOL_{}_TAKER_FEE_BPS", symbol_id);
        if let Ok(v) = std::env::var(&taker_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.taker_fee_bps[sid] = parsed;
            }
        }
        let maker_key =
            format!("RSX_SYMBOL_{}_MAKER_FEE_BPS", symbol_id);
        if let Ok(v) = std::env::var(&maker_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.maker_fee_bps[sid] = parsed;
            }
        }
        let im_key = format!(
            "RSX_SYMBOL_{}_INITIAL_MARGIN_RATE",
            symbol_id
        );
        if let Ok(v) = std::env::var(&im_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.margin.symbol_params[sid].initial_margin_rate =
                    parsed;
            }
        }
        let mm_key = format!(
            "RSX_SYMBOL_{}_MAINTENANCE_MARGIN_RATE",
            symbol_id
        );
        if let Ok(v) = std::env::var(&mm_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.margin.symbol_params[sid]
                    .maintenance_margin_rate = parsed;
            }
        }
        let lev_key = format!(
            "RSX_SYMBOL_{}_MAX_LEVERAGE",
            symbol_id
        );
        if let Ok(v) = std::env::var(&lev_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.margin.symbol_params[sid].max_leverage =
                    parsed;
            }
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

    /// LIQUIDATOR.md §9. Process socialized loss event.
    /// Deduct loss from insurance fund, persist events.
    fn process_socialized_loss(
        &mut self,
        loss: &crate::liquidation::SocializedLoss,
        now_ns: u64,
    ) {
        let loss_amount = loss.qty * loss.price;

        // Ensure insurance fund exists for symbol
        let fund = self
            .insurance_funds
            .entry(loss.symbol_id)
            .or_insert_with(|| {
                InsuranceFund::new(loss.symbol_id, 0)
            });

        // Deduct from insurance fund (balance can go negative)
        fund.deduct(loss_amount);

        // Clone fund for persistence before releasing borrow
        let fund_snapshot = fund.clone();

        warn!(
            "socialized loss: user={} symbol={} \
             round={} qty={} price={} loss={}",
            loss.user_id,
            loss.symbol_id,
            loss.round,
            loss.qty,
            loss.price,
            loss_amount
        );

        // Persist liquidation event with status=3 (socialized)
        self.push_persist(
            PersistEvent::LiquidationEvent(
                LiquidationEventRecord {
                    user_id: loss.user_id,
                    symbol_id: loss.symbol_id,
                    round: loss.round,
                    side: loss.side,
                    price: loss.price,
                    qty: loss.qty,
                    slippage_bps: 0,
                    status: 3,
                    timestamp_ns: now_ns,
                },
            ),
        );

        // Persist updated insurance fund
        self.push_persist(
            PersistEvent::InsuranceFund(fund_snapshot),
        );
    }

    /// One iteration of the main loop.
    /// Priority: fills > orders > mark > bbo > funding.
    pub fn run_once(
        &mut self,
        rings: &mut ShardRings,
        now_secs: u64,
    ) {
        // Backpressure: stall if persist ring full
        if self.backpressured {
            if let Some(ref p) = self.persist_producer {
                if p.slots() > 0 {
                    self.backpressured = false;
                } else {
                    return;
                }
            } else {
                self.backpressured = false;
            }
        }

        // 1. Drain all fills (highest priority)
        for consumer in &mut rings.fill_consumers {
            while let Ok(fill) = consumer.pop() {
                self.process_fill(&fill);
            }
        }

        // 1b. Process pending liquidations
        let now_ns = now_secs * 1_000_000_000;
        let (liq_orders, socialized_losses) = {
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

        // Process socialized losses (LIQUIDATOR.md §9)
        for loss in &socialized_losses {
            self.process_socialized_loss(loss, now_ns);
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

    pub fn set_replica_tip_producer(
        &mut self,
        producer: Producer<(u32, u64)>,
    ) {
        self.replica_tip_producer = Some(producer);
    }

    pub fn push_tip_to_replica(
        &mut self,
        symbol_id: u32,
        tip: u64,
    ) {
        if let Some(ref mut p) = self.replica_tip_producer {
            if p.push((symbol_id, tip)).is_err() {
                warn!(
                    "replica tip ring full for sym {}",
                    symbol_id
                );
            }
        }
    }

    pub fn is_replica(&self) -> bool {
        self.replica_state.is_some()
    }

    pub fn buffer_fill_for_replica(
        &mut self,
        fill: FillEvent,
    ) {
        if let Some(ref mut r) = self.replica_state {
            r.buffer_fill(fill);
        }
    }

    pub fn apply_tip_from_main(
        &mut self,
        symbol_id: u32,
        tip: u64,
    ) {
        if let Some(ref mut r) = self.replica_state {
            r.apply_tip(symbol_id, tip);
            let fills =
                r.drain_fills_up_to_tip(symbol_id);
            for fill in fills {
                self.process_fill(&fill);
            }
        }
    }

    pub fn promote_from_replica(
        &mut self,
    ) -> Vec<FillEvent> {
        if let Some(ref mut r) = self.replica_state {
            let fills = r.drain_all_up_to_tips();
            for fill in &fills {
                self.process_fill(fill);
            }
            info!(
                shard_id = self.shard_id,
                fills_applied = fills.len(),
                "promoted from replica"
            );
            fills
        } else {
            Vec::new()
        }
    }

    pub fn replica_buffered_count(&self) -> usize {
        self.replica_state
            .as_ref()
            .map(|r| r.total_buffered())
            .unwrap_or(0)
    }
}

fn order_key(order_id_hi: u64, order_id_lo: u64) -> u128 {
    ((order_id_hi as u128) << 64) | order_id_lo as u128
}
