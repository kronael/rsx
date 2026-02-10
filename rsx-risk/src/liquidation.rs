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
    halted_symbols: Vec<bool>,
    base_delay_ns: u64,
    base_slip_bps: i64,
    max_rounds: u32,
}

impl LiquidationEngine {
    pub fn new(
        base_delay_ns: u64,
        base_slip_bps: i64,
        max_rounds: u32,
    ) -> Self {
        Self {
            active: Vec::new(),
            halted_symbols: Vec::new(),
            base_delay_ns,
            base_slip_bps,
            max_rounds,
        }
    }

    /// Halt liquidation for a symbol (e.g. on ORDER_FAILED).
    pub fn halt_symbol(&mut self, symbol_id: u32) {
        let idx = symbol_id as usize;
        if idx >= self.halted_symbols.len() {
            self.halted_symbols.resize(idx + 1, false);
        }
        self.halted_symbols[idx] = true;
    }

    /// Resume liquidation for a symbol.
    pub fn resume_symbol(&mut self, symbol_id: u32) {
        let idx = symbol_id as usize;
        if idx < self.halted_symbols.len() {
            self.halted_symbols[idx] = false;
        }
    }

    pub fn is_halted(&self, symbol_id: u32) -> bool {
        let idx = symbol_id as usize;
        idx < self.halted_symbols.len()
            && self.halted_symbols[idx]
    }

    pub fn enqueue(
        &mut self,
        user_id: u32,
        symbol_id: u32,
        now_ns: u64,
    ) {
        let already = self.active.iter().any(|s| {
            s.user_id == user_id
                && s.symbol_id == symbol_id
                && s.status != LiquidationStatus::Done
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
        let mut orders = Vec::new();
        let mut socialized = Vec::new();
        for state in &mut self.active {
            if state.status != LiquidationStatus::Active {
                continue;
            }
            let sid = state.symbol_id as usize;
            if sid < self.halted_symbols.len()
                && self.halted_symbols[sid]
            {
                continue;
            }
            let delay =
                state.round as u64 * self.base_delay_ns;
            if state.last_order_ns != 0
                && now_ns < state.last_order_ns + delay
            {
                continue;
            }
            let net_qty =
                get_position_fn(state.user_id, state.symbol_id);
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
                continue;
            }

            let slip = state.round as i64
                * state.round as i64
                * self.base_slip_bps;
            let (side, price) = if net_qty > 0 {
                (1u8, mark * (10_000 - slip) / 10_000)
            } else {
                (0u8, mark * (10_000 + slip) / 10_000)
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
        (orders, socialized)
    }

    pub fn cancel_if_recovered(
        &mut self,
        user_id: u32,
        symbol_id: u32,
    ) {
        self.active.retain(|s| {
            !(s.user_id == user_id
                && s.symbol_id == symbol_id)
        });
    }

    pub fn remove_done(&mut self) {
        self.active.retain(|s| {
            s.status != LiquidationStatus::Done
        });
    }

    pub fn is_in_liquidation(
        &self,
        user_id: u32,
        symbol_id: u32,
    ) -> bool {
        self.active.iter().any(|s| {
            s.user_id == user_id
                && s.symbol_id == symbol_id
                && s.status == LiquidationStatus::Active
        })
    }
}
