use crate::account::Account;
use crate::config::ShardConfig;
use crate::funding;
use crate::funding::FundingConfig;
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

    pub fills_processed: u64,
    pub orders_processed: u64,
    persist_producer: Option<Producer<PersistEvent>>,
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
            fills_processed: 0,
            orders_processed: 0,
            persist_producer: None,
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
        if fill.preamble.seq <= self.tips[sid] {
            return;
        }

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
                fill.preamble.seq,
            );
            let fee = calculate_fee(
                fill.qty,
                fill.price,
                self.taker_fee_bps[sid],
            );
            self.accounts
                .get_mut(&fill.taker_user_id)
                .unwrap()
                .deduct_fee(fee);
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
                fill.preamble.seq,
            );
            let fee = calculate_fee(
                fill.qty,
                fill.price,
                self.maker_fee_bps[sid],
            );
            self.accounts
                .get_mut(&fill.maker_user_id)
                .unwrap()
                .deduct_fee(fee);
            self.update_exposure(
                fill.maker_user_id,
                fill.symbol_id,
            );
        }

        self.tips[sid] = fill.preamble.seq;
        self.fills_processed += 1;

        // Persist fill + updated positions + tip
        self.push_persist(PersistEvent::Fill(PersistFill {
            symbol_id: fill.symbol_id,
            taker_user_id: fill.taker_user_id,
            maker_user_id: fill.maker_user_id,
            price: fill.price,
            qty: fill.qty,
            taker_fee: 0,
            maker_fee: 0,
            taker_side: fill.taker_side,
            seq: fill.preamble.seq,
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
            seq: fill.preamble.seq,
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

    /// RISK.md §6. Pre-trade risk check.
    pub fn process_order(
        &mut self,
        order: &OrderRequest,
    ) -> OrderResponse {
        if !self.user_in_shard(order.user_id) {
            return OrderResponse::Rejected {
                user_id: order.user_id,
                reason: RejectReason::NotInShard,
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
                }
            }
            Err(reason) => OrderResponse::Rejected {
                user_id: order.user_id,
                reason,
            },
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

        // 2. Drain orders
        while let Ok(order) = rings.order_consumer.pop() {
            let resp = self.process_order(&order);
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
