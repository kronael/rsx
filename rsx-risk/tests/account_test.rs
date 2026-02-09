use rsx_risk::account::Account;

#[test]
fn freeze_increases_frozen_margin() {
    let mut a = Account::new(1, 10000);
    a.freeze_margin(500);
    assert_eq!(a.frozen_margin, 500);
}

#[test]
fn release_decreases_frozen_margin() {
    let mut a = Account::new(1, 10000);
    a.freeze_margin(500);
    a.release_margin(200);
    assert_eq!(a.frozen_margin, 300);
}

#[test]
fn deduct_fee_reduces_collateral() {
    let mut a = Account::new(1, 10000);
    a.deduct_fee(100);
    assert_eq!(a.collateral, 9900);
}

#[test]
fn deduct_negative_fee_credits_collateral() {
    let mut a = Account::new(1, 10000);
    a.deduct_fee(-50);
    assert_eq!(a.collateral, 10050);
}

#[test]
fn version_increments_on_each_op() {
    let mut a = Account::new(1, 10000);
    assert_eq!(a.version, 0);
    a.freeze_margin(100);
    assert_eq!(a.version, 1);
    a.release_margin(50);
    assert_eq!(a.version, 2);
    a.deduct_fee(10);
    assert_eq!(a.version, 3);
}
