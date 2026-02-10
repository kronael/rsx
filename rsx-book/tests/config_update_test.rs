use rsx_book::book::Orderbook;
use rsx_types::SymbolConfig;

#[test]
fn update_config_changes_book_config() {
    let initial_config = SymbolConfig {
        symbol_id: 1,
        tick_size: 1,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 8,
    };

    let mut book = Orderbook::new(initial_config, 1024, 50_000);
    assert_eq!(book.config.tick_size, 1);
    assert_eq!(book.config.lot_size, 1000);

    let new_config = SymbolConfig {
        symbol_id: 1,
        tick_size: 10,
        lot_size: 10_000,
        price_decimals: 6,
        qty_decimals: 6,
    };

    book.update_config(new_config);
    assert_eq!(book.config.tick_size, 10);
    assert_eq!(book.config.lot_size, 10_000);
    assert_eq!(book.config.price_decimals, 6);
    assert_eq!(book.config.qty_decimals, 6);
}

#[test]
fn update_config_preserves_book_state() {
    let config = SymbolConfig {
        symbol_id: 1,
        tick_size: 1,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 8,
    };

    let mut book = Orderbook::new(config, 1024, 50_000);
    let initial_seq = book.sequence;

    let new_config = SymbolConfig {
        symbol_id: 1,
        tick_size: 10,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 8,
    };

    book.update_config(new_config);
    assert_eq!(book.sequence, initial_seq);
}
