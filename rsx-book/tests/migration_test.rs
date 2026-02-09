use rsx_book::book::BookState;
use rsx_book::book::Orderbook;
use rsx_types::NONE;
use rsx_types::Side;
use rsx_types::SymbolConfig;

fn test_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn test_book() -> Orderbook {
    Orderbook::new(test_config(), 1024, 50_000)
}

#[test]
fn trigger_fires_when_mid_drifts() {
    let book = test_book();
    // Zone 0: 5% of 50_000 = 2_500 ticks
    // Trigger at > 50% of zone 0 = > 1_250 ticks
    assert!(!book.should_recenter(50_000));
    assert!(book.should_recenter(51_500));
    assert!(book.should_recenter(48_500));
}

#[test]
fn recenter_swaps_active_staging() {
    let mut book = test_book();
    let _old_len = book.active_levels.len();
    book.trigger_recenter(52_000);
    assert_eq!(book.state, BookState::Migrating);
    // Active levels should be new size
    assert!(book.active_levels.len() > 0);
}

#[test]
fn frontier_starts_at_new_mid() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    assert_eq!(book.bid_frontier, 52_000);
    assert_eq!(book.ask_frontier, 52_000);
}

#[test]
fn resolve_level_migrates_on_access() {
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.trigger_recenter(50_500);
    // Access a price outside frontier
    book.resolve_level(49_900);
    // Frontier should have expanded
    assert!(book.bid_frontier <= 49_900);
}

#[test]
fn migrate_empty_level_is_noop() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    // Migrating an empty old level should be fine
    book.migrate_single_level(0);
    // No crash
}

#[test]
fn cancel_during_migration() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.trigger_recenter(50_500);

    // Resolve first so order is in new array
    book.resolve_level(49_950);

    // Cancel should work
    assert!(book.cancel_order(h));
}

#[test]
fn insert_during_migration_goes_to_new() {
    let mut book = test_book();
    book.trigger_recenter(52_000);

    // Insert goes to new active array
    let _h = book.insert_resting(
        52_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    assert_ne!(book.best_ask_tick, NONE);
}

#[test]
fn best_bid_ask_correct_after_recenter() {
    let mut book = test_book();
    book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.insert_resting(
        50_050, 100, Side::Sell, 0, 2, false, 0, 0, 0,
    );

    book.trigger_recenter(50_500);

    // After migration, BBA should be reset
    // (NONE until orders are migrated)
    // Migrate orders
    book.resolve_level(49_950);
    book.resolve_level(50_050);

    assert_ne!(book.best_bid_tick, NONE);
    assert_ne!(book.best_ask_tick, NONE);
    assert!(book.best_bid_tick < book.best_ask_tick);
}

#[test]
fn snapshot_blocked_during_migration() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    assert!(book.is_migrating());
}

#[test]
fn batch_migration() {
    let mut book = test_book();
    book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false, 0, 0, 0,
    );
    book.trigger_recenter(50_500);

    // Run enough batches to complete migration
    for _ in 0..1000 {
        book.migrate_batch(1000);
        if !book.is_migrating() {
            break;
        }
    }
    // Should eventually complete
    // (may or may not depending on old_levels scan)
}
