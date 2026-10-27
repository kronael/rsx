use crate::symbols::build_table;
use crate::symbols::is_status_token;
use crate::symbols::sort_entries;
use rsx_types::SymbolConfig;

fn config(symbol_id: u32) -> SymbolConfig {
    SymbolConfig {
        symbol_id,
        price_decimals: 6,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 100_000,
    }
}

#[test]
fn entries_sort_by_symbol_id() {
    let table = build_table([config(14), config(1), config(10)]);
    let ids: Vec<u32> = sort_entries(&table)
        .into_iter()
        .map(|entry| entry.config.symbol_id)
        .collect();
    assert_eq!(ids, [1, 10, 14]);
}

#[test]
fn status_is_a_lowercase_word() {
    assert!(is_status_token("trading"));
    assert!(is_status_token("close_only"));
    assert!(!is_status_token(""));
    assert!(!is_status_token("Trading"));
    assert!(!is_status_token("a\"b"));
}
