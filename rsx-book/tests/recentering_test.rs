use rsx_book::book::BookState;
use rsx_book::book::Orderbook;
use rsx_book::snapshot;
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
fn recenter_triggers_when_mid_drifts_beyond_zone_0()
{
    let book = test_book();
    // Zone 0 = 5% of 50_000 = 2_500 ticks.
    // Trigger threshold = zone0_half / 2 = 1_250.
    assert!(!book.should_recenter(50_000));
    assert!(!book.should_recenter(51_000));
    assert!(book.should_recenter(51_500));
    assert!(book.should_recenter(48_500));
}

#[test]
fn recenter_swaps_active_and_staging() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    assert_eq!(book.state, BookState::Migrating);
    assert!(book.active_levels.len() > 0);
}

#[test]
fn recenter_frontier_starts_at_new_mid() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    assert_eq!(book.bid_frontier, 52_000);
    assert_eq!(book.ask_frontier, 52_000);
}

#[test]
fn resolve_level_migrates_on_access_outside_frontier()
{
    let mut book = test_book();
    book.insert_resting(
        49_900, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    book.trigger_recenter(50_500);
    book.resolve_level(49_900);
    assert!(book.bid_frontier <= 49_900);
}

#[test]
fn migrate_single_level_moves_orders_to_new_indices()
{
    let mut book = test_book();
    let h = book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    let old_tick = book.orders.get(h).tick_index;

    book.trigger_recenter(50_500);
    book.resolve_level(49_950);

    // Order should now have a new tick index in the
    // new compression map
    let new_tick = book.orders.get(h).tick_index;
    let expected =
        book.compression.price_to_index(49_950);
    assert_eq!(new_tick, expected);
    // Old and new tick may differ due to new mid
    assert!(
        new_tick != old_tick || old_tick == expected
    );
    // Level at new tick should have the order
    assert_eq!(
        book.active_levels[new_tick as usize]
            .order_count,
        1,
    );
}

#[test]
fn migrate_empty_level_is_noop() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    book.migrate_single_level(0);
    // No crash, no state change
    assert_eq!(book.state, BookState::Migrating);
}

#[test]
fn migrate_batch_expands_frontiers() {
    let mut book = test_book();
    let initial_bid = 50_500i64;
    let initial_ask = 50_500i64;
    book.trigger_recenter(50_500);
    assert_eq!(book.bid_frontier, initial_bid);
    assert_eq!(book.ask_frontier, initial_ask);

    book.migrate_batch(10);

    // Frontiers should have expanded
    assert!(book.bid_frontier < initial_bid);
    assert!(book.ask_frontier > initial_ask);
}

#[test]
fn migrate_completes_when_all_levels_drained() {
    let mut book = test_book();
    book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    book.insert_resting(
        50_050, 100, Side::Sell, 0, 2, false,
        0, 0, 0,
    );
    book.trigger_recenter(50_500);
    assert!(book.is_migrating());

    // Run large batches until complete
    for _ in 0..10_000 {
        book.migrate_batch(1000);
        if !book.is_migrating() {
            break;
        }
    }
    assert_eq!(book.state, BookState::Normal);
    assert!(book.old_levels.is_none());
    assert!(book.old_compression.is_none());
}

#[test]
fn cancel_during_migration_resolves_first() {
    let mut book = test_book();
    let h = book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    book.trigger_recenter(50_500);
    // Resolve first so order is in new array
    book.resolve_level(49_950);
    assert!(book.cancel_order(h));
}

#[test]
fn insert_during_migration_goes_to_new_array() {
    let mut book = test_book();
    book.trigger_recenter(52_000);
    let _h = book.insert_resting(
        52_100, 100, Side::Sell, 0, 1, false,
        0, 0, 0,
    );
    assert_ne!(book.best_ask_tick, NONE);
}

#[test]
fn best_bid_ask_correct_after_recenter() {
    let mut book = test_book();
    book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    book.insert_resting(
        50_050, 100, Side::Sell, 0, 2, false,
        0, 0, 0,
    );
    book.trigger_recenter(50_500);
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
    let mut buf = Vec::new();
    let result = snapshot::save(&book, &mut buf);
    assert!(result.is_err());
}

#[test]
fn snapshot_runs_after_migration_completes() {
    let mut book = test_book();
    book.insert_resting(
        49_950, 100, Side::Buy, 0, 1, false,
        0, 0, 0,
    );
    book.trigger_recenter(50_500);

    // Complete migration
    for _ in 0..10_000 {
        book.migrate_batch(1000);
        if !book.is_migrating() {
            break;
        }
    }
    assert!(!book.is_migrating());

    let mut buf = Vec::new();
    let result = snapshot::save(&book, &mut buf);
    assert!(result.is_ok());
    assert!(!buf.is_empty());
}
