//! Tests for `recenter_now` — the eager recenter path the live ME uses.
//! A recenter must fully migrate before the next order is matched: lazy
//! per-order migration is not correct for a marketable order (its crossing
//! liquidity can lie outside the migrated band -> missed fills -> crossed
//! book). These pin: matching stays correct across a recenter, no order is
//! lost, and the book never crosses.

use rsx_book::book::BookState;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rsx_types::NONE;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

fn taker(price: i64, qty: i64, side: Side, user_id: u32) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::GTC,
        user_id,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 0,
        order_id_hi: 0,
        order_id_lo: user_id as u64,
    }
}

fn active_orders(book: &Orderbook) -> u32 {
    (0..book.orders.len())
        .filter(|&i| book.orders.get(i).is_active())
        .count() as u32
}

/// Orders reachable by walking every level. Equals `active_orders` only
/// when NOT migrating (all orders live in `active_levels`), so it catches
/// an order dropped from the level lists but left active (an orphan/leak).
fn linked_orders(book: &Orderbook) -> u32 {
    let mut n = 0;
    for lvl in &book.active_levels {
        let mut c = lvl.head;
        while c != NONE {
            n += 1;
            c = book.orders.get(c).next;
        }
    }
    n
}

fn assert_uncrossed(book: &Orderbook) {
    if book.best_bid_px != 0 && book.best_ask_px != 0 {
        assert!(
            book.best_bid_px < book.best_ask_px,
            "crossed book: bid {} >= ask {}",
            book.best_bid_px,
            book.best_ask_px,
        );
    }
}

/// The hazard: a marketable order arriving right after a recenter must
/// still cross the (now migrated) resting liquidity, not skip it and rest
/// above it. `recenter_now` migrates everything up front, so it does.
#[test]
fn cross_after_recenter_fills_migrated_liquidity() {
    let mut book = Orderbook::new(config(), 4096, 1_000_000);
    book.insert_resting(1_000_010, 100, Side::Sell, 0, 1, false, 0, 0, 1);
    book.insert_resting(1_000_020, 100, Side::Sell, 0, 2, false, 0, 0, 2);
    book.insert_resting(999_990, 100, Side::Buy, 0, 3, false, 0, 0, 3);

    // Drift the mid up 30k (> half of zone 0 = 25k) and recenter.
    book.recenter_now(1_030_000);
    assert_eq!(book.state, BookState::Normal, "recenter_now must complete");
    assert!(book.old_levels.is_none());
    // All three resting orders survived the migration.
    assert_eq!(active_orders(&book), 3);
    assert_eq!(linked_orders(&book), 3, "an order was orphaned by recenter");
    assert_eq!(book.best_ask_px, 1_000_010);
    assert_eq!(book.best_bid_px, 999_990);

    // Marketable buy crosses BOTH sells (below its limit) — these levels
    // sit far below the new mid, i.e. exactly the band a lazy scheme would
    // have missed.
    let mut buy = taker(1_000_025, 150, Side::Buy, 4);
    process_new_order(&mut book, &mut buy);
    let fills: Vec<i64> = book
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::Fill { qty, .. } => Some(qty.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        fills.iter().sum::<i64>(),
        150,
        "missed migrated liquidity: {fills:?}"
    );
    assert_eq!(buy.remaining_qty, 0);
    assert_uncrossed(&book);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo) as u64) as i64
    }
}

/// Random flow with periodic recenters must keep the book uncrossed, never
/// leak/orphan a slab slot, and (since recenter_now completes) always land
/// back in Normal with no old arrays held.
#[test]
fn random_flow_with_recenters_stays_consistent() {
    let mut mid = 1_000_000_i64;
    let mut book = Orderbook::new(config(), 200_000, mid);
    let mut rng = Rng(0xF00D_1234_5678_9ABC);
    let mut live: Vec<u32> = Vec::new();
    let mut oid = 1u64;

    for step in 0..4000 {
        // Occasionally drift the mid and recenter through the live path.
        if step % 500 == 499 {
            mid += 40_000; // > half of zone 0
            book.recenter_now(mid);
            assert_eq!(book.state, BookState::Normal);
            assert!(book.old_levels.is_none());
            assert_eq!(
                linked_orders(&book),
                active_orders(&book),
                "recenter orphaned an order at step {step}",
            );
            assert_uncrossed(&book);
        }

        if !live.is_empty() && rng.next().is_multiple_of(3) {
            let k = (rng.next() as usize) % live.len();
            let h = live.swap_remove(k);
            book.cancel_order(h);
        } else {
            let buy = rng.next() & 1 == 0;
            let off = match rng.next() % 10 {
                0..=6 => rng.range(1, 30_000),
                _ => rng.range(30_000, 200_000),
            };
            let price = if buy { mid - off } else { mid + off };
            let side = if buy { Side::Buy } else { Side::Sell };
            let mut o = taker(price, rng.range(1, 400), side, (oid % 64) as u32 + 1);
            process_new_order(&mut book, &mut o);
            for ev in book.events() {
                if let Event::OrderInserted { handle, .. } = ev {
                    live.push(*handle);
                }
            }
            oid += 1;
        }

        // Slab accounting holds every step (no double-free / lost slot).
        assert_eq!(
            book.orders.len(),
            book.orders.free_count() + active_orders(&book),
            "slab leak at step {step}",
        );
        if step % 11 == 0 {
            assert_uncrossed(&book);
        }
    }
    assert_uncrossed(&book);
    assert_eq!(linked_orders(&book), active_orders(&book));
}
