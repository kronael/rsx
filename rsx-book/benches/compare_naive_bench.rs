//! rsx-book vs the obvious baseline: `BTreeMap<price, VecDeque<order>>`.
//!
//! This is the comparison the CEO audit asked for ("so what, vs the
//! obvious thing everyone would reach for first"). Same ops, same
//! depths, same Criterion harness (`harness.rs`, shared with
//! `book_bench`/`deep_book_bench`), same box, same RNG seed per depth
//! so both books hold statistically-identical content.
//!
//! Ops (mirroring the existing rsx-book benches so the comparison is
//! apples-to-apples):
//! - `match_clear`  — an aggressor that consumes exactly the touch
//!   level's quantity, clearing it, then a replenish. This is the
//!   level-clear case the O(slots) scan bug hit before the occupancy
//!   bitmap fix (`da9a2b4`) — mirrors `match_ioc_vs_1k_asks`.
//! - `insert_cancel` — insert one order, then cancel it (net-neutral
//!   depth), mirrors `insert_cancel_depth`.
//! - `cancel`       — cancel from a pre-built pool of resting orders,
//!   refilling untimed between batches, mirrors `cancel_depth`.
//!
//! The naive book is a textbook implementation: `BTreeMap<i64,
//! VecDeque<NaiveOrder>>` per side, `HashMap<order_id, (Side, i64)>` to
//! locate an order's price level for cancel (linear scan within the
//! level's VecDeque — no slab, no compression map, no occupancy
//! bitmap). This is what "the obvious thing" looks like, not a
//! strawman: BTreeMap already gives O(log n) removal + O(log n)
//! next-best, so it never had rsx-book's pre-fix O(slots) bug — the
//! honest baseline is a *tree*, not a linear scan.
//!
//! Run: cargo bench -p rsx-book --bench compare_naive_bench

#[path = "harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::TimeInForce;

const DEPTHS: [u64; 3] = [100, 1_000, 10_000];

// --- naive book -------------------------------------------------------

struct NaiveOrder {
    id: u64,
    qty: i64,
}

/// `BTreeMap<price, VecDeque<order>>` per side + a locator map for
/// cancel-by-id. The textbook baseline: no slab, no compression map,
/// no occupancy bitmap.
struct NaiveBook {
    bids: BTreeMap<i64, VecDeque<NaiveOrder>>,
    asks: BTreeMap<i64, VecDeque<NaiveOrder>>,
    locate: HashMap<u64, (Side, i64)>,
    next_id: u64,
}

impl NaiveBook {
    fn new() -> Self {
        NaiveBook {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locate: HashMap::new(),
            next_id: 1,
        }
    }

    fn side_map(
        &mut self,
        side: Side,
    ) -> &mut BTreeMap<i64, VecDeque<NaiveOrder>> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    /// Insert a resting order on `side`. Returns its id.
    fn insert_side(
        &mut self,
        price: i64,
        qty: i64,
        side: Side,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.side_map(side)
            .entry(price)
            .or_default()
            .push_back(NaiveOrder { id, qty });
        self.locate.insert(id, (side, price));
        id
    }

    /// Cancel by id: locate the level, linear-scan the VecDeque
    /// (naive — no per-order handle), remove it, drop the level entry
    /// if it's now empty.
    fn cancel(&mut self, id: u64) -> bool {
        let Some((side, price)) = self.locate.remove(&id)
        else {
            return false;
        };
        let map = self.side_map(side);
        let Some(dq) = map.get_mut(&price) else {
            return false;
        };
        let Some(pos) =
            dq.iter().position(|o| o.id == id)
        else {
            return false;
        };
        dq.remove(pos);
        if dq.is_empty() {
            map.remove(&price);
        }
        true
    }

    /// Match an aggressor against the opposite side, consuming FIFO
    /// from the best level, popping the level when it clears. Returns
    /// filled qty.
    fn match_order(
        &mut self,
        side: Side,
        mut qty: i64,
        limit_price: i64,
    ) -> i64 {
        let mut filled = 0_i64;
        let opposite = match side {
            Side::Buy => &mut self.asks,
            Side::Sell => &mut self.bids,
        };
        loop {
            if qty <= 0 {
                break;
            }
            let best = match side {
                // aggressor buys: best ask = lowest price
                Side::Buy => opposite
                    .iter()
                    .next()
                    .map(|(&p, _)| p),
                // aggressor sells: best bid = highest price
                Side::Sell => opposite
                    .iter()
                    .next_back()
                    .map(|(&p, _)| p),
            };
            let Some(px) = best else {
                break;
            };
            let crosses = match side {
                Side::Buy => px <= limit_price,
                Side::Sell => px >= limit_price,
            };
            if !crosses {
                break;
            }
            let dq = opposite.get_mut(&px).expect(
                "INVARIANT: price key came from this map",
            );
            let front = dq.front_mut().expect(
                "INVARIANT: non-empty level has a front",
            );
            let take = qty.min(front.qty);
            front.qty -= take;
            qty -= take;
            filled += take;
            if front.qty == 0 {
                let done = dq.pop_front().expect(
                    "INVARIANT: just matched the front",
                );
                self.locate.remove(&done.id);
                if dq.is_empty() {
                    opposite.remove(&px);
                }
            }
        }
        filled
    }
}

fn build_naive(n: u64) -> NaiveBook {
    let mut book = NaiveBook::new();
    // Same seed as harness::build(n) — identical RNG stream, so both
    // books hold statistically-matched content at a given depth.
    let mut rng =
        harness::Rng::new(0x1234_5678_9abc_def0 ^ n);
    for _ in 0..n {
        let (p, q, s) = harness::draw_resting(&mut rng);
        book.insert_side(p, q, s);
    }
    book
}

// --- match_clear: aggressor exactly clears the touch level ------------

/// rsx-book: prefill N asks at consecutive prices (qty 100 each), buy
/// exactly 100 against the touch (clears it), replenish. Mirrors
/// `match_ioc_vs_1k_asks` in `book_bench.rs`, parameterized by depth.
fn bench_match_clear_rsx(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("match_clear_rsx");
    g.throughput(Throughput::Elements(1));
    for &n in &DEPTHS {
        g.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, &n| {
                let mut book = Orderbook::new(
                    harness::config(),
                    (n + 8192) as u32,
                    50000,
                );
                for i in 0..n {
                    book.insert_resting(
                        50001 + i as i64,
                        100,
                        Side::Sell,
                        0,
                        2,
                        false,
                        1000,
                        0,
                        i,
                    );
                }
                let mut seq = n + 10_000;
                b.iter(|| {
                    let mut order = IncomingOrder {
                        price: 50001,
                        qty: 100,
                        remaining_qty: 100,
                        side: Side::Buy,
                        tif: TimeInForce::IOC,
                        user_id: 1,
                        reduce_only: false,
                        post_only: false,
                        timestamp_ns: 1000,
                        order_id_hi: 0,
                        order_id_lo: seq,
                    };
                    process_new_order(
                        black_box(&mut book),
                        black_box(&mut order),
                    );
                    seq += 1;
                    // Replenish the just-cleared touch level.
                    book.insert_resting(
                        50001, 100, Side::Sell, 0, 2,
                        false, 1000, 0, seq,
                    );
                    seq += 1;
                });
            },
        );
    }
    g.finish();
}

/// Naive BTreeMap book: same shape — N asks at consecutive prices (qty
/// 100 each), buy exactly 100 against the touch (clears + removes the
/// BTreeMap entry), replenish.
fn bench_match_clear_naive(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("match_clear_naive");
    g.throughput(Throughput::Elements(1));
    for &n in &DEPTHS {
        g.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, &n| {
                let mut book = NaiveBook::new();
                for i in 0..n {
                    book.insert_side(
                        50001 + i as i64,
                        100,
                        Side::Sell,
                    );
                }
                b.iter(|| {
                    let filled = book.match_order(
                        black_box(Side::Buy),
                        black_box(100),
                        black_box(50001),
                    );
                    black_box(filled);
                    // Replenish the just-cleared touch level.
                    book.insert_side(50001, 100, Side::Sell);
                });
            },
        );
    }
    g.finish();
}

// --- insert_cancel: insert one order then cancel it (net-neutral) -----

fn bench_insert_cancel_rsx(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("insert_cancel_rsx");
    g.throughput(Throughput::Elements(1));
    for &n in &DEPTHS {
        let mut book = harness::build(n);
        let mut rng = harness::Rng::new(0xBEEF ^ n);
        g.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, _| {
                b.iter(|| {
                    let (p, q, s) =
                        harness::draw_resting(&mut rng);
                    let h = book.insert_resting(
                        p, q, s, 0, 1, false, 1, 0, 0,
                    );
                    black_box(book.cancel_order(h));
                });
            },
        );
    }
    g.finish();
}

fn bench_insert_cancel_naive(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("insert_cancel_naive");
    g.throughput(Throughput::Elements(1));
    for &n in &DEPTHS {
        let mut book = build_naive(n);
        let mut rng = harness::Rng::new(0xBEEF ^ n);
        g.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, _| {
                b.iter(|| {
                    let (p, q, s) =
                        harness::draw_resting(&mut rng);
                    let id =
                        book.insert_side(p, q, s);
                    black_box(book.cancel(id));
                });
            },
        );
    }
    g.finish();
}

// --- cancel: pool of resting orders, cancel + untimed refill -----------

fn bench_cancel_rsx(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("cancel_rsx");
    g.throughput(Throughput::Elements(1));
    for &n in &DEPTHS {
        let pool = (n as usize).clamp(1, 4096);
        g.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, _| {
                b.iter_custom(|iters| {
                    let mut book = harness::build(n);
                    let mut rng =
                        harness::Rng::new(0xCA9 ^ n);
                    let mut handles: Vec<u32> = (0
                        ..pool)
                        .map(|_| {
                            let (p, q, s) =
                                harness::draw_resting(
                                    &mut rng,
                                );
                            book.insert_resting(
                                p, q, s, 0, 1, false,
                                1, 0, 0,
                            )
                        })
                        .collect();
                    let mut done = 0_u64;
                    let mut elapsed =
                        std::time::Duration::ZERO;
                    while done < iters {
                        let batch = ((iters - done)
                            as usize)
                            .min(handles.len());
                        let start =
                            std::time::Instant::now();
                        for h in
                            handles.iter().take(batch)
                        {
                            black_box(
                                book.cancel_order(*h),
                            );
                        }
                        elapsed += start.elapsed();
                        done += batch as u64;
                        for slot in handles
                            .iter_mut()
                            .take(batch)
                        {
                            let (p, q, s) =
                                harness::draw_resting(
                                    &mut rng,
                                );
                            *slot = book.insert_resting(
                                p, q, s, 0, 1, false,
                                1, 0, 0,
                            );
                        }
                    }
                    elapsed
                });
            },
        );
    }
    g.finish();
}

fn bench_cancel_naive(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("cancel_naive");
    g.throughput(Throughput::Elements(1));
    for &n in &DEPTHS {
        let pool = (n as usize).clamp(1, 4096);
        g.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, _| {
                b.iter_custom(|iters| {
                    let mut book = build_naive(n);
                    let mut rng =
                        harness::Rng::new(0xCA9 ^ n);
                    let mut ids: Vec<u64> = (0..pool)
                        .map(|_| {
                            let (p, q, s) =
                                harness::draw_resting(
                                    &mut rng,
                                );
                            book.insert_side(p, q, s)
                        })
                        .collect();
                    let mut done = 0_u64;
                    let mut elapsed =
                        std::time::Duration::ZERO;
                    while done < iters {
                        let batch = ((iters - done)
                            as usize)
                            .min(ids.len());
                        let start =
                            std::time::Instant::now();
                        for id in ids.iter().take(batch)
                        {
                            black_box(
                                book.cancel(*id),
                            );
                        }
                        elapsed += start.elapsed();
                        done += batch as u64;
                        for slot in
                            ids.iter_mut().take(batch)
                        {
                            let (p, q, s) =
                                harness::draw_resting(
                                    &mut rng,
                                );
                            *slot =
                                book.insert_side(p, q, s);
                        }
                    }
                    elapsed
                });
            },
        );
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets =
        bench_match_clear_rsx,
        bench_match_clear_naive,
        bench_insert_cancel_rsx,
        bench_insert_cancel_naive,
        bench_cancel_rsx,
        bench_cancel_naive,
}
criterion_main!(benches);
