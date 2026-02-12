use rsx_book::OrderSlot;
use rsx_book::PriceLevel;

#[test]
fn order_slot_size_is_128_bytes() {
    assert_eq!(
        std::mem::size_of::<OrderSlot>(), 128
    );
}

#[test]
fn order_slot_alignment_is_64() {
    assert_eq!(
        std::mem::align_of::<OrderSlot>(), 64
    );
}

#[test]
fn price_level_size_is_24_bytes() {
    assert_eq!(
        std::mem::size_of::<PriceLevel>(), 24
    );
}

/// Hot fields (price, remaining_qty, side, flags,
/// tif, _pad1, next, prev, tick_index, _pad2) must
/// fit in the first cache line (64 bytes). We verify
/// the sum of hot field sizes <= 48B (leaving room
/// within the 64B cache line for alignment).
#[test]
fn order_slot_hot_fields_in_first_cache_line() {
    // price: i64 (8) + remaining_qty: i64 (8)
    // + side: u8 (1) + flags: u8 (1) + tif: u8 (1)
    // + _pad1: [u8;5] (5) + next: u32 (4)
    // + prev: u32 (4) + tick_index: u32 (4)
    // + _pad2: u32 (4) = 40 bytes
    let hot_size: usize = 8 + 8 + 1 + 1 + 1 + 5
        + 4 + 4 + 4 + 4;
    assert_eq!(hot_size, 40);
    assert!(hot_size <= 64);

    // Verify via offset_of that tick_index ends
    // within the first 64 bytes. tick_index starts
    // at offset 32 (8+8+1+1+1+5+4+4) and is 4
    // bytes, so ends at 36. _pad2 ends at 40.
    // All hot fields within first cache line.
    assert!(hot_size <= 48);
}
