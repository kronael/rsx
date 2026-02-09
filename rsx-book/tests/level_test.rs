use rsx_book::level::PriceLevel;

#[test]
fn price_level_size_is_24_bytes() {
    assert_eq!(
        std::mem::size_of::<PriceLevel>(),
        24
    );
}
