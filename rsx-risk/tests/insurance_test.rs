/// LIQUIDATOR.md §9. Insurance fund unit tests.

use rsx_risk::insurance::InsuranceFund;

// -- basic operations --

#[test]
fn new_fund_initialized() {
    let fund = InsuranceFund::new(100, 50_000_000);
    assert_eq!(fund.symbol_id, 100);
    assert_eq!(fund.balance, 50_000_000);
    assert_eq!(fund.version, 0);
}

#[test]
fn deduct_reduces_balance() {
    let mut fund = InsuranceFund::new(100, 100_000);
    fund.deduct(10_000);
    assert_eq!(fund.balance, 90_000);
    assert_eq!(fund.version, 1);
}

#[test]
fn add_increases_balance() {
    let mut fund = InsuranceFund::new(100, 100_000);
    fund.add(20_000);
    assert_eq!(fund.balance, 120_000);
    assert_eq!(fund.version, 1);
}

#[test]
fn version_increments_on_deduct() {
    let mut fund = InsuranceFund::new(100, 50_000);
    assert_eq!(fund.version, 0);
    fund.deduct(1000);
    assert_eq!(fund.version, 1);
    fund.deduct(500);
    assert_eq!(fund.version, 2);
}

#[test]
fn version_increments_on_add() {
    let mut fund = InsuranceFund::new(100, 50_000);
    fund.add(1000);
    assert_eq!(fund.version, 1);
    fund.add(2000);
    assert_eq!(fund.version, 2);
}

// -- edge cases --

#[test]
fn deduct_can_go_negative() {
    let mut fund = InsuranceFund::new(100, 10_000);
    fund.deduct(15_000);
    assert_eq!(fund.balance, -5_000);
    assert_eq!(fund.version, 1);
}

#[test]
fn add_to_negative_balance() {
    let mut fund = InsuranceFund::new(100, -10_000);
    fund.add(15_000);
    assert_eq!(fund.balance, 5_000);
    assert_eq!(fund.version, 1);
}

#[test]
fn zero_initial_balance() {
    let fund = InsuranceFund::new(100, 0);
    assert_eq!(fund.balance, 0);
}

#[test]
fn negative_initial_balance() {
    let fund = InsuranceFund::new(100, -50_000);
    assert_eq!(fund.balance, -50_000);
}

#[test]
fn multiple_deductions() {
    let mut fund = InsuranceFund::new(100, 100_000);
    fund.deduct(10_000);
    fund.deduct(20_000);
    fund.deduct(5_000);
    assert_eq!(fund.balance, 65_000);
    assert_eq!(fund.version, 3);
}

#[test]
fn multiple_additions() {
    let mut fund = InsuranceFund::new(100, 0);
    fund.add(10_000);
    fund.add(20_000);
    fund.add(5_000);
    assert_eq!(fund.balance, 35_000);
    assert_eq!(fund.version, 3);
}

#[test]
fn mixed_add_and_deduct() {
    let mut fund = InsuranceFund::new(100, 50_000);
    fund.add(10_000);
    fund.deduct(5_000);
    fund.add(3_000);
    fund.deduct(8_000);
    assert_eq!(fund.balance, 50_000);
    assert_eq!(fund.version, 4);
}

// -- large values --

#[test]
fn large_balance() {
    let mut fund = InsuranceFund::new(100, 1_000_000_000_000);
    fund.deduct(100_000_000_000);
    assert_eq!(fund.balance, 900_000_000_000);
}

#[test]
fn very_large_deduction() {
    let mut fund = InsuranceFund::new(100, i64::MAX / 2);
    fund.deduct(i64::MAX / 4);
    assert!(fund.balance > 0);
}

// -- socialized loss simulation --

#[test]
fn socialized_loss_scenario() {
    // User with 10 BTC at 50k, underwater
    // Liquidation fails, socialize 10 BTC
    let mut fund = InsuranceFund::new(100, 100_000_000);
    let loss_qty = 10;
    let price = 50_000;
    let loss = loss_qty * price;
    fund.deduct(loss);
    assert_eq!(fund.balance, 100_000_000 - 500_000);
}

#[test]
fn multiple_socialized_losses() {
    let mut fund = InsuranceFund::new(100, 1_000_000);
    fund.deduct(100_000);
    fund.deduct(50_000);
    fund.deduct(75_000);
    assert_eq!(fund.balance, 775_000);
    assert_eq!(fund.version, 3);
}

#[test]
fn insurance_fund_depleted() {
    let mut fund = InsuranceFund::new(100, 100_000);
    fund.deduct(150_000);
    assert_eq!(fund.balance, -50_000);
    assert!(fund.balance < 0);
}

// -- clone and default --

#[test]
fn clone_produces_identical_fund() {
    let fund = InsuranceFund::new(100, 50_000);
    let cloned = fund.clone();
    assert_eq!(cloned.symbol_id, fund.symbol_id);
    assert_eq!(cloned.balance, fund.balance);
    assert_eq!(cloned.version, fund.version);
}

#[test]
fn default_produces_zeroed_fund() {
    let fund = InsuranceFund::default();
    assert_eq!(fund.symbol_id, 0);
    assert_eq!(fund.balance, 0);
    assert_eq!(fund.version, 0);
}
