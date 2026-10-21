use rsx_gateway::convert::*;

#[test]
fn validate_tick_alignment_pass() {
    assert!(validate_tick_alignment(5000000, 100));
}

#[test]
fn validate_tick_alignment_fail() {
    assert!(!validate_tick_alignment(5000001, 100));
}

#[test]
fn validate_lot_alignment_pass() {
    assert!(validate_lot_alignment(1500, 1));
    assert!(validate_lot_alignment(150, 10));
}

#[test]
fn validate_lot_alignment_fail() {
    assert!(!validate_lot_alignment(155, 10));
}
