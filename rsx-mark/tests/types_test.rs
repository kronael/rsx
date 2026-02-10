use std::mem::align_of;
use std::mem::size_of;

use rsx_mark::types::MarkPriceEvent;
use rsx_mark::types::SourcePrice;
use rsx_mark::types::SymbolMarkState;

#[test]
fn mark_price_event_size_is_64() {
    assert_eq!(size_of::<MarkPriceEvent>(), 64);
}

#[test]
fn mark_price_event_alignment_is_64() {
    assert_eq!(align_of::<MarkPriceEvent>(), 64);
}

#[test]
fn source_price_is_copy() {
    let sp = SourcePrice {
        source_id: 0,
        price: 100,
        timestamp_ns: 1000,
    };
    let sp2 = sp;
    assert_eq!(sp.price, sp2.price);
}

#[test]
fn mark_state_initial_all_none() {
    let state = SymbolMarkState::new();
    for slot in &state.sources {
        assert!(slot.is_none());
    }
    assert_eq!(state.mark_price, 0);
    assert_eq!(state.source_mask, 0);
    assert_eq!(state.source_count, 0);
}

#[test]
fn mark_state_source_mask_correct_bitmask() {
    use rsx_mark::aggregator::aggregate;
    let mut state = SymbolMarkState::new();
    let now = 1_000_000_000u64;
    let update = SourcePrice {
        source_id: 2,
        price: 5000,
        timestamp_ns: now,
    };
    aggregate(&mut state, update, now, 0);
    assert_eq!(state.source_mask, 0b100);
}

#[test]
fn mark_state_source_count_matches_fresh() {
    use rsx_mark::aggregator::aggregate;
    let mut state = SymbolMarkState::new();
    let now = 1_000_000_000u64;
    for i in 0..3u8 {
        let update = SourcePrice {
            source_id: i,
            price: 5000 + i as i64,
            timestamp_ns: now,
        };
        aggregate(&mut state, update, now, 0);
    }
    assert_eq!(state.source_count, 3);
}

#[test]
fn mark_state_mark_price_updated_on_aggregate() {
    use rsx_mark::aggregator::aggregate;
    let mut state = SymbolMarkState::new();
    let now = 1_000_000_000u64;
    let update = SourcePrice {
        source_id: 0,
        price: 42000,
        timestamp_ns: now,
    };
    aggregate(&mut state, update, now, 0);
    assert_eq!(state.mark_price, 42000);
}
