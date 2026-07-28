use rsx_book::order::OrderSlot;

#[test]
fn order_slot_size_is_128_bytes() {
    assert_eq!(std::mem::size_of::<OrderSlot>(), 128);
}

#[test]
fn order_slot_alignment_is_64() {
    assert_eq!(std::mem::align_of::<OrderSlot>(), 64);
}

#[test]
fn order_slot_active_flag() {
    let mut order = OrderSlot::default();
    assert!(!order.is_active());
    order.set_active(true);
    assert!(order.is_active());
    order.set_active(false);
    assert!(!order.is_active());
}

#[test]
fn order_slot_reduce_only_flag() {
    let mut order = OrderSlot::default();
    assert!(!order.is_reduce_only());
    order.flags |= 2;
    assert!(order.is_reduce_only());
}
