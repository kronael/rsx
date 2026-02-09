use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_types::SymbolConfig;

fn main() {
    std::panic::set_hook(Box::new(|_| {
        std::process::exit(0);
    }));

    // TODO: load config from TOML first arg
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    };

    let mut book =
        Orderbook::new(config, 1024, 50_000);

    loop {
        // TODO: try_pop from SPSC ring
        // if let Some(order) = try_pop() {
        //     process_new_order(&mut book, &mut order);
        //     drain_events(&book);
        // } else
        if book.is_migrating() {
            book.migrate_batch(100);
        }
        // bare busy-spin: no yield, dedicated core
    }
}

fn drain_events(book: &Orderbook) {
    for event in book.events() {
        // TODO: push to SPSC ring for downstream
        // consumers (risk, mktdata, recorder)
    }
}
