use rsx_risk::account::Account;

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
fn version_increments_on_fee() {
    let mut a = Account::new(1, 10000);
    assert_eq!(a.version, 0);
    a.deduct_fee(10);
    assert_eq!(a.version, 1);
    a.deduct_fee(-5);
    assert_eq!(a.version, 2);
}
