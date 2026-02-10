use rsx_risk::account::Account;
use rsx_risk::margin::ExposureIndex;
use rsx_risk::margin::MarginState;
use rsx_risk::margin::PortfolioMargin;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::position::Position;

fn make_pm(n: usize) -> PortfolioMargin {
    PortfolioMargin {
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000, // 10%
                maintenance_margin_rate: 500, // 5%
                max_leverage: 10,
            };
            n
        ],
    }
}

#[test]
fn margin_check_detects_undercollateralized() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 100, 1); // long 100@100
    // mark drops to 10 -> upnl = 100*(10-100) = -9000
    // equity = 1000 + (-9000) = -8000
    // notional = 100*10 = 1000, mm = 50
    let a = Account::new(1, 1000);
    let state = pm.calculate(&a, &[&p], &[10]);
    assert!(state.equity < state.maintenance_margin);
    assert!(pm.needs_liquidation(&state));
}

#[test]
fn margin_check_passes_healthy_account() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    let a = Account::new(1, 100_000);
    let state = pm.calculate(&a, &[&p], &[100]);
    // equity=100000, im=100, mm=50
    assert!(state.equity > state.initial_margin);
    assert!(!pm.needs_liquidation(&state));
}

#[test]
fn margin_check_borderline_not_liquidated() {
    // equity == maintenance_margin -> NOT liquidated
    // needs_liquidation uses strict < not <=
    let pm = make_pm(1);
    let state = MarginState {
        equity: 50,
        maintenance_margin: 50,
        ..Default::default()
    };
    assert!(!pm.needs_liquidation(&state));
}

#[test]
fn exposure_index_tracks_users() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(2, 10);
    idx.add_user(2, 20);
    let users = idx.users_for_symbol(2);
    assert!(users.contains(&10));
    assert!(users.contains(&20));
    assert_eq!(users.len(), 2);
}

#[test]
fn exposure_index_removes_on_close() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(1, 42);
    assert_eq!(idx.users_for_symbol(1), &[42]);
    idx.remove_user(1, 42);
    assert!(idx.users_for_symbol(1).is_empty());
}
