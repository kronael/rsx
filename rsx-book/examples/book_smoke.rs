//! Standalone quick-start for rsx-book: build a book, rest a maker,
//! cross it with a taker (a fill), then cancel a resting order —
//! printing the events the book emits at each step.
//!
//! Run it: `cargo run -p rsx-book --example book_smoke`.
//!
//! All prices/quantities are raw i64 units (tick_size = lot_size = 1
//! here, so the raw number IS the human number). A real deployment
//! converts at the API boundary; the book only ever sees i64.

use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::Event;
use rsx_book::Orderbook;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

/// Print every event the book emitted for the order just processed.
/// `process_new_order` resets the buffer at the start of each call,
/// so `book.events()` holds exactly this order's events.
fn dump(step: &str, book: &Orderbook) {
    println!("\n{step}");
    for event in book.events() {
        match event {
            Event::Fill { price, qty, maker_handle, .. } => {
                println!(
                    "  Fill        px={} qty={} maker_handle={maker_handle}",
                    price.0, qty.0
                );
            }
            Event::OrderInserted { handle, price, qty, .. } => {
                println!(
                    "  OrderInserted  handle={handle} px={} qty={}",
                    price.0, qty.0
                );
            }
            Event::OrderCancelled { handle, remaining_qty, .. } => {
                println!(
                    "  OrderCancelled handle={handle} remaining={}",
                    remaining_qty.0
                );
            }
            Event::OrderDone { handle, filled_qty, remaining_qty, .. } => {
                println!(
                    "  OrderDone   handle={handle} filled={} remaining={}",
                    filled_qty.0, remaining_qty.0
                );
            }
            Event::OrderFailed { reason, .. } => {
                println!("  OrderFailed reason={reason}");
            }
            Event::BBO { bid_px, bid_qty, ask_px, ask_qty, .. } => {
                println!(
                    "  BBO         bid {}x{}  ask {}x{}",
                    bid_px.0, bid_qty.0, ask_px.0, ask_qty.0
                );
            }
        }
    }
}

/// Pull the slab handle out of the OrderInserted event a resting order
/// produced — the caller keeps it to cancel the order later (O(1)).
fn resting_handle(book: &Orderbook) -> u32 {
    for event in book.events() {
        if let Event::OrderInserted { handle, .. } = event {
            return *handle;
        }
    }
    panic!("expected the order to rest");
}

fn main() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    };
    // One book per symbol. 1M slab slots; mid_price seeds the
    // compression map (raw i64 units).
    let mut book = Orderbook::new(config, 1_000_000, 100);

    // 1. Rest a maker: sell 10 @ 100 (GTC, no cross yet).
    let mut maker_a = IncomingOrder {
        price: 100,
        qty: 10,
        remaining_qty: 10,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 1,
    };
    process_new_order(&mut book, &mut maker_a);
    dump("[1] rest maker A: sell 10 @ 100", &book);

    // 2. Rest a second maker: sell 5 @ 101 — the one we'll cancel.
    let mut maker_b = IncomingOrder {
        price: 101,
        qty: 5,
        remaining_qty: 5,
        side: Side::Sell,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 2,
    };
    process_new_order(&mut book, &mut maker_b);
    dump("[2] rest maker B: sell 5 @ 101", &book);
    let handle_b = resting_handle(&book);

    // 3. Cross with a taker: buy 10 @ 100 — fully fills against maker A.
    let mut taker = IncomingOrder {
        price: 100,
        qty: 10,
        remaining_qty: 10,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: 3,
    };
    process_new_order(&mut book, &mut taker);
    dump("[3] cross taker: buy 10 @ 100 (fills maker A)", &book);

    // 4. Cancel resting maker B by its slab handle (O(1) unlink).
    let cancelled = book.cancel_order(handle_b);
    println!("\n[4] cancel maker B (handle={handle_b}) -> {cancelled}");
}
