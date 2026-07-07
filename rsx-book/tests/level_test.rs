use rsx_book::level::PriceLevel;

#[test]
fn price_level_size_is_32_bytes() {
    // head/tail/total_qty/order_count + per-side bid_count/ask_count.
    assert_eq!(std::mem::size_of::<PriceLevel>(), 32);
}
