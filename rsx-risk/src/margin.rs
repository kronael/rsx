use crate::account::Account;
use crate::position::Position;
use crate::risk_utils::calculate_fee;
use crate::types::OrderRequest;
use crate::types::RejectReason;

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
}

pub struct PortfolioMargin {
    pub symbol_params: Vec<SymbolRiskParams>,
}

impl PortfolioMargin {
    /// RISK.md §3. Full portfolio margin calculation.
    pub fn calculate(
        &self,
        account: &Account,
        positions: &[&Position],
        mark_prices: &[i64],
    ) -> MarginState {
        let mut upnl = 0i64;
        let mut im = 0i64;
        let mut mm = 0i64;
        for pos in positions {
            let sid = pos.symbol_id as usize;
            let mark = mark_prices[sid];
            upnl += pos.unrealized_pnl(mark);
            let notional = pos.notional(mark);
            let params = &self.symbol_params[sid];
            im += (notional as i128
                * params.initial_margin_rate as i128
                / 10_000) as i64;
            mm += (notional as i128
                * params.maintenance_margin_rate as i128
                / 10_000) as i64;
        }
        let equity = account.collateral + upnl;
        let available =
            equity - im - account.frozen_margin;
        MarginState {
            equity,
            unrealized_pnl: upnl,
            initial_margin: im,
            maintenance_margin: mm,
            available_margin: available,
        }
    }

    /// RISK.md §6. Pre-trade check.
    pub fn check_order(
        &self,
        account: &Account,
        positions: &[&Position],
        order: &OrderRequest,
        mark_prices: &[i64],
        taker_fee_bps: i64,
    ) -> Result<i64, RejectReason> {
        if order.is_liquidation {
            return Ok(0);
        }
        if order.reduce_only {
            return Ok(0);
        }
        let state =
            self.calculate(account, positions, mark_prices);
        let notional_128 = order.price as i128
            * order.qty as i128;
        let order_notional = i64::try_from(notional_128)
            .unwrap_or(i64::MAX);
        let sid = order.symbol_id as usize;
        let params = &self.symbol_params[sid];
        let order_im = (order_notional as i128
            * params.initial_margin_rate as i128
            / 10_000) as i64;
        let order_fee = calculate_fee(
            order.qty,
            order.price,
            taker_fee_bps,
        );
        let margin_needed = order_im + order_fee;
        if state.available_margin < margin_needed {
            return Err(RejectReason::InsufficientMargin);
        }
        Ok(margin_needed)
    }

    /// RISK.md §7.
    pub fn needs_liquidation(
        &self,
        state: &MarginState,
    ) -> bool {
        state.equity < state.maintenance_margin
    }
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

    pub fn add_user(
        &mut self,
        symbol_idx: usize,
        user_id: u32,
    ) {
        let users = &mut self.exposure[symbol_idx];
        if !users.contains(&user_id) {
            users.push(user_id);
        }
    }

    pub fn remove_user(
        &mut self,
        symbol_idx: usize,
        user_id: u32,
    ) {
        let users = &mut self.exposure[symbol_idx];
        users.retain(|&u| u != user_id);
    }

    pub fn users_for_symbol(
        &self,
        symbol_idx: usize,
    ) -> &[u32] {
        &self.exposure[symbol_idx]
    }
}
