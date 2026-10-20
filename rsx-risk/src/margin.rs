use crate::account::Account;
use crate::position::Position;
use crate::risk_utils::calculate_fee;
use crate::types::OrderRequest;
use crate::types::RejectReason;
use tracing::error;

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
