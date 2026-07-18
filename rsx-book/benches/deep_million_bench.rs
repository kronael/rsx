use std::collections::HashSet;
use std::fs;
use std::panic::AssertUnwindSafe;
use std::time::Instant;

use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::MAX_EVENTS;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rsx_types::NONE;

const ORDERS: u64 = 1_000_000;
const LEVELS: u64 = 1_000;
const CASCADE_FILLS: i64 = 30_000;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn buy(qty: i64, oid: u64) -> IncomingOrder {
    IncomingOrder {
        price: 1_001_000,
        qty,
        remaining_qty: qty,
        side: Side::Buy,
        tif: TimeInForce::IOC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 1,
        order_id_lo: oid,
    }
}

fn seed(book: &mut Orderbook, orders: u64) {
    let per_level = orders / LEVELS;
    let extra = orders % LEVELS;
    let mut oid = 1;
    for level in 0..LEVELS {
        let at_level = per_level + u64::from(level < extra);
        for _ in 0..at_level {
            book.insert_resting(
                1_000_001 + level as i64,
                1,
                Side::Sell,
                TimeInForce::GTC as u8,
                1,
                false,
                oid,
                0,
                oid,
            );
            oid += 1;
        }
    }
}

fn verify_oversize_panics() {
    let fills = (MAX_EVENTS / 2) as u64;
    let mut book = Orderbook::new(config(), fills as u32, 1_000_000);
    seed(&mut book, fills);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut order = buy(fills as i64, 1);
        process_new_order(&mut book, &mut order);
    }));
    std::panic::set_hook(hook);
    let payload = result.expect_err("oversize cascade must panic");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        message.contains("INVARIANT: ME event buffer overflow")
            && message.contains("MAX_EVENTS=65536"),
        "unexpected panic: {message}",
    );
    println!("oversize_boundary=panic_verified fills={fills} MAX_EVENTS={MAX_EVENTS}");
}

fn main() {
    assert!(
        CASCADE_FILLS as usize * 2 + 2 < MAX_EVENTS,
        "bounded cascade must stay below MAX_EVENTS",
    );

    let started = Instant::now();
    let mut book = Orderbook::new(config(), ORDERS as u32, 1_000_000);
    seed(&mut book, ORDERS);
    assert_eq!(book.orders.len(), ORDERS as u32);
    assert_eq!(book.orders.free_count(), 0);
    assert_eq!(book.current_bbo(), (0, 0, 1_000_001, 1_000));

    let mut next_maker = 1_u64;
    let mut makers = HashSet::with_capacity(ORDERS as usize);
    let mut fill_count = 0_u64;
    let mut fill_qty = 0_i64;
    let mut taker = 1_u64;

    while fill_count < ORDERS {
        let qty = CASCADE_FILLS.min((ORDERS - fill_count) as i64);
        let mut order = buy(qty, taker);
        process_new_order(&mut book, &mut order);
        assert_eq!(order.remaining_qty, 0);

        let mut cascade_fills = 0_i64;
        for event in book.events() {
            if let Event::Fill {
                maker_order_id_lo,
                qty,
                ..
            } = event
            {
                assert_eq!(*maker_order_id_lo, next_maker, "FIFO maker order");
                assert!(makers.insert(*maker_order_id_lo), "duplicate maker fill");
                next_maker += 1;
                cascade_fills += 1;
                fill_count += 1;
                fill_qty += qty.0;
            }
        }
        assert_eq!(cascade_fills, qty);
        taker += 1;
    }

    assert_eq!(fill_count, ORDERS);
    assert_eq!(fill_qty, ORDERS as i64);
    assert_eq!(makers.len(), ORDERS as usize);
    assert_eq!(next_maker, ORDERS + 1);
    assert_eq!(book.best_bid_tick, NONE);
    assert_eq!(book.best_ask_tick, NONE);
    assert_eq!(book.current_bbo(), (0, 0, 0, 0));
    assert_eq!(book.orders.len(), ORDERS as u32);
    assert_eq!(book.orders.free_count(), ORDERS as u32);
    assert_eq!(book.orders.alloc(), (ORDERS - 1) as u32, "slab reuse");

    verify_oversize_panics();
    let peak_rss_kb = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or(0);
    println!(
        "deep_million orders={ORDERS} cascades={} max_fills_per_cascade={CASCADE_FILLS} elapsed_ms={} peak_rss_kb={peak_rss_kb}",
        taker - 1,
        started.elapsed().as_millis(),
    );
}
