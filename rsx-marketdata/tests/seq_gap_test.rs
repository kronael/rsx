use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;

fn make_state() -> MarketDataState {
    let config = SymbolConfig {
        symbol_id: 0,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    MarketDataState::new(4, config, 100, 50000)
}

#[test]
fn first_seq_no_gap() {
    let mut state = make_state();
    assert!(!state.check_seq(0, 1));
    assert_eq!(state.gap_count(), 0);
}

#[test]
fn sequential_no_gap() {
    let mut state = make_state();
    assert!(!state.check_seq(0, 1));
    assert!(!state.check_seq(0, 2));
    assert!(!state.check_seq(0, 3));
    assert_eq!(state.gap_count(), 0);
}

#[test]
fn gap_detected() {
    let mut state = make_state();
    assert!(!state.check_seq(0, 1));
    assert!(state.check_seq(0, 5));
    assert_eq!(state.gap_count(), 1);
}

#[test]
fn duplicate_ignored() {
    let mut state = make_state();
    assert!(!state.check_seq(0, 1));
    assert!(!state.check_seq(0, 1));
    assert_eq!(state.gap_count(), 0);
}

#[test]
fn different_symbols_independent() {
    let mut state = make_state();
    assert!(!state.check_seq(0, 1));
    assert!(!state.check_seq(1, 1));
    assert!(state.check_seq(0, 5));
    assert!(!state.check_seq(1, 2));
    assert_eq!(state.gap_count(), 1);
}

#[test]
fn gap_resumes_tracking() {
    let mut state = make_state();
    assert!(!state.check_seq(0, 1));
    assert!(state.check_seq(0, 5));
    assert!(!state.check_seq(0, 6));
    assert_eq!(state.gap_count(), 1);
}
