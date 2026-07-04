//! Unified cross-match: rsx-book vs every level-touch-capable order-book
//! contender in one extensible harness.
//!
//! See `rsx-book/compare/README.md` for the full writeup, fairness
//! caveats, and numbers (own-box cited numbers vs same-box benched here).
//!
//! ## Design
//!
//! One trait, [`BenchBook`], captures the level-touch op set every
//! contender is driven through: `insert` / `reduce` / `cancel` a resting
//! qty at an abstract price level, `best` price read, and an optional
//! `match_touch` for contenders that do real order-level FIFO matching
//! (not just L2 aggregation). Each contender is one `impl BenchBook`
//! below. Adding a future contender is: implement the trait, add one
//! line to `all_contenders()`.
//!
//! `level` is an ABSTRACT index (0..M), not a literal price — each
//! contender maps it to whatever native price representation it needs
//! (rsx-book: fixed-point i64 ticks; hftbacktest/lob/orderbook: f64
//! dollars). Bid levels and ask levels always map to disjoint,
//! non-crossing price ranges (bids below a fixed mid, asks at/above it)
//! so contenders with a global bid<ask invariant (`orderbook` crate)
//! never see a crossed insert from this synthetic stream.
//!
//! Capability flags (`supports_reduce`, `supports_best`, `supports_match`)
//! are the "fairness by construction" mechanism: a contender that can't
//! do an op is excluded from that op's bench group entirely and the run
//! prints an explicit "N/A (unsupported)" line for it — never a faked
//! number.
//!
//! Run: cargo bench -p rsx-book --bench compare_all_bench

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use rsx_types::Side;

// ── Trait ───────────────────────────────────────────────────────────────

/// Level-touch interface every contender implements. `level` is an
/// abstract 0-based index into a side's price ladder (0 = closest to
/// the touch), not a literal price.
trait BenchBook {
    fn insert(&mut self, side: Side, level: i64, qty: i64) -> bool;
    fn reduce(&mut self, side: Side, level: i64, qty: i64) -> bool;
    fn cancel(&mut self, side: Side, level: i64) -> bool;
    /// Best price on a side, in the contender's own internal
    /// representation (not cross-contender comparable as a *value* —
    /// only the cost of reading it is compared).
    fn best(&self, side: Side) -> Option<i64>;
    /// Marketable order against the given side's resting liquidity.
    /// Returns filled qty, or `None` if this contender has no real
    /// order-level matching algorithm (L2-aggregated depth trackers
    /// like hftbacktest, or feed-recording books like `orderbook`,
    /// never support this).
    fn match_touch(&mut self, side: Side, qty: i64) -> Option<i64> {
        let _ = (side, qty);
        None
    }
    fn supports_reduce(&self) -> bool {
        true
    }
    fn supports_best(&self) -> bool {
        true
    }
    fn supports_match(&self) -> bool {
        false
    }
}

type Factory = fn() -> Box<dyn BenchBook>;

fn all_contenders() -> Vec<(&'static str, Factory)> {
    vec![
        ("rsx_book", || Box::new(rsx_book_impl::RsxBook::new())),
        ("naive_btree", || Box::new(naive_btree_impl::NaiveBtree::new())),
        (
            "hftbacktest_btree",
            || Box::new(hftbacktest_impl::HftBtree::new()),
        ),
        (
            "hftbacktest_hashmap",
            || Box::new(hftbacktest_impl::HftHashMap::new()),
        ),
        ("lob", || Box::new(lob_impl::LobBook::new())),
        // "orderbook" (inv2004, crates.io "orderbook" v0.1.9) is
        // deliberately NOT in this list: it panics with an internal
        // usize underflow (index out of bounds) the moment one side of
        // the book empties completely then refills — exactly the
        // insert->cancel->insert cycle every op stream in this file
        // drives. See compare/orderbook-inv2004.md for the repro and
        // why this isn't "dodge a fair test", it's an upstream crash.
    ]
}

// ── rsx-book (real) ────────────────────────────────────────────────────

mod rsx_book_impl {
    use super::BenchBook;
    use rsx_book::book::Orderbook;
    use rsx_book::matching::process_new_order;
    use rsx_book::matching::IncomingOrder;
    use rsx_types::Side;
    use rsx_types::SymbolConfig;
    use rsx_types::TimeInForce;
    use rsx_types::NONE;

    const MID: i64 = 50_000;
    const LEVELS_MAX: usize = 4096;

    fn config() -> SymbolConfig {
        SymbolConfig {
            symbol_id: 1,
            price_decimals: 2,
            qty_decimals: 3,
            tick_size: 1,
            lot_size: 1,
        }
    }

    fn price(side: Side, level: i64) -> i64 {
        match side {
            Side::Buy => MID - 1 - level,
            Side::Sell => MID + level,
        }
    }

    pub struct RsxBook {
        book: Orderbook,
        bid_handles: Vec<Option<u32>>,
        ask_handles: Vec<Option<u32>>,
    }

    impl RsxBook {
        pub fn new() -> Self {
            RsxBook {
                book: Orderbook::new(config(), 1_000_000, MID),
                bid_handles: vec![None; LEVELS_MAX],
                ask_handles: vec![None; LEVELS_MAX],
            }
        }

        fn handles(&mut self, side: Side) -> &mut Vec<Option<u32>> {
            match side {
                Side::Buy => &mut self.bid_handles,
                Side::Sell => &mut self.ask_handles,
            }
        }
    }

    impl BenchBook for RsxBook {
        fn insert(&mut self, side: Side, level: i64, qty: i64) -> bool {
            let px = price(side, level);
            let h = self.book.insert_resting(
                px, qty, side, 0, 1, false, 1000, 0, level as u64,
            );
            self.handles(side)[level as usize] = Some(h);
            true
        }

        fn reduce(&mut self, side: Side, level: i64, qty: i64) -> bool {
            match self.handles(side)[level as usize] {
                Some(h) => self.book.modify_order_qty_down(h, qty),
                None => false,
            }
        }

        fn cancel(&mut self, side: Side, level: i64) -> bool {
            match self.handles(side)[level as usize].take() {
                Some(h) => self.book.cancel_order(h),
                None => false,
            }
        }

        fn best(&self, side: Side) -> Option<i64> {
            let t = match side {
                Side::Buy => self.book.best_bid_tick,
                Side::Sell => self.book.best_ask_tick,
            };
            if t == NONE { None } else { Some(t as i64) }
        }

        fn match_touch(&mut self, side: Side, qty: i64) -> Option<i64> {
            let px = match side {
                Side::Buy => MID + 1_000_000,
                Side::Sell => MID - 1_000_000,
            };
            let mut o = IncomingOrder {
                price: px,
                qty,
                remaining_qty: qty,
                side,
                tif: TimeInForce::IOC,
                user_id: 1,
                reduce_only: false,
                post_only: false,
                timestamp_ns: 1,
                order_id_hi: 0,
                order_id_lo: 0,
            };
            process_new_order(&mut self.book, &mut o);
            Some(qty - o.remaining_qty)
        }

        fn supports_match(&self) -> bool {
            true
        }
    }
}

// ── Naive BTreeMap baseline (built here; the "obvious thing" floor) ────

mod naive_btree_impl {
    use super::BenchBook;
    use rsx_types::Side;
    use std::collections::BTreeMap;
    use std::collections::VecDeque;

    const MID: i64 = 1_000_000;

    fn price(side: Side, level: i64) -> i64 {
        match side {
            Side::Buy => MID - 1 - level,
            Side::Sell => MID + level,
        }
    }

    pub struct NaiveBtree {
        bids: BTreeMap<i64, VecDeque<i64>>,
        asks: BTreeMap<i64, VecDeque<i64>>,
    }

    impl NaiveBtree {
        pub fn new() -> Self {
            NaiveBtree { bids: BTreeMap::new(), asks: BTreeMap::new() }
        }

        fn map(&mut self, side: Side) -> &mut BTreeMap<i64, VecDeque<i64>> {
            match side {
                Side::Buy => &mut self.bids,
                Side::Sell => &mut self.asks,
            }
        }
    }

    impl BenchBook for NaiveBtree {
        fn insert(&mut self, side: Side, level: i64, qty: i64) -> bool {
            let px = price(side, level);
            self.map(side).insert(px, VecDeque::from([qty]));
            true
        }

        fn reduce(&mut self, side: Side, level: i64, qty: i64) -> bool {
            let px = price(side, level);
            match self.map(side).get_mut(&px) {
                Some(q) if q.len() == 1 => {
                    q[0] = qty;
                    true
                }
                _ => false,
            }
        }

        fn cancel(&mut self, side: Side, level: i64) -> bool {
            let px = price(side, level);
            self.map(side).remove(&px).is_some()
        }

        fn best(&self, side: Side) -> Option<i64> {
            match side {
                Side::Buy => self.bids.keys().next_back().copied(),
                Side::Sell => self.asks.keys().next().copied(),
            }
        }

        /// Taker `side` sweeps the OPPOSITE side's levels in price-time
        /// priority (ascending for asks when buying, descending for
        /// bids when selling) — the same walk rsx-book's `match_*`
        /// benches exercise, just against a BTreeMap+VecDeque book.
        fn match_touch(&mut self, side: Side, qty: i64) -> Option<i64> {
            let mut remaining = qty;
            let opposite = match side {
                Side::Buy => &mut self.asks,
                Side::Sell => &mut self.bids,
            };
            let levels: Vec<i64> = match side {
                Side::Buy => opposite.keys().copied().collect(),
                Side::Sell => opposite.keys().rev().copied().collect(),
            };
            for px in levels {
                if remaining == 0 {
                    break;
                }
                let empty = {
                    let q = opposite.get_mut(&px).expect("level in keys snapshot");
                    while remaining > 0 {
                        let Some(front) = q.front_mut() else { break };
                        if *front <= remaining {
                            remaining -= *front;
                            q.pop_front();
                        } else {
                            *front -= remaining;
                            remaining = 0;
                        }
                    }
                    q.is_empty()
                };
                if empty {
                    opposite.remove(&px);
                }
            }
            Some(qty - remaining)
        }

        fn supports_match(&self) -> bool {
            true
        }
    }
}

// ── hftbacktest depth (L2-aggregated, no FIFO/matching) ─────────────────

mod hftbacktest_impl {
    use super::BenchBook;
    use hftbacktest::depth::BTreeMarketDepth;
    use hftbacktest::depth::HashMapMarketDepth;
    use hftbacktest::depth::L2MarketDepth;
    use rsx_types::Side;

    const MID: i64 = 1_000_000;

    fn price(side: Side, level: i64) -> f64 {
        (match side {
            Side::Buy => MID - 1 - level,
            Side::Sell => MID + level,
        }) as f64
    }

    macro_rules! impl_hft_depth {
        ($name:ident, $inner:ty) => {
            pub struct $name {
                depth: $inner,
            }

            impl $name {
                pub fn new() -> Self {
                    $name { depth: <$inner>::new(1.0, 1.0) }
                }
            }

            impl BenchBook for $name {
                fn insert(&mut self, side: Side, level: i64, qty: i64) -> bool {
                    let px = price(side, level);
                    match side {
                        Side::Buy => self.depth.update_bid_depth(px, qty as f64, 1000),
                        Side::Sell => self.depth.update_ask_depth(px, qty as f64, 1000),
                    };
                    true
                }

                fn reduce(&mut self, side: Side, level: i64, qty: i64) -> bool {
                    self.insert(side, level, qty)
                }

                fn cancel(&mut self, side: Side, level: i64) -> bool {
                    let px = price(side, level);
                    match side {
                        Side::Buy => self.depth.update_bid_depth(px, 0.0, 1000),
                        Side::Sell => self.depth.update_ask_depth(px, 0.0, 1000),
                    };
                    true
                }

                fn best(&self, side: Side) -> Option<i64> {
                    let t = match side {
                        Side::Buy => self.depth.best_bid_tick,
                        Side::Sell => self.depth.best_ask_tick,
                    };
                    if t == hftbacktest::depth::INVALID_MIN
                        || t == hftbacktest::depth::INVALID_MAX
                    {
                        None
                    } else {
                        Some(t)
                    }
                }
            }
        };
    }

    impl_hft_depth!(HftBtree, BTreeMarketDepth);
    impl_hft_depth!(HftHashMap, HashMapMarketDepth);
}

// ── lob (rafalpiotrowski/lob-rs): real order-level book, no reduce/best
//    API surface exposed publicly ────────────────────────────────────────

mod lob_impl {
    use super::BenchBook;
    use lob::Oid;
    use lob::Order;
    use lob::OrderBook;
    use lob::OrderSide;
    use lob::Timestamp;
    use rsx_types::Side;
    use std::collections::HashMap;

    const MID: f64 = 1_000_000.0;

    fn price(side: Side, level: i64) -> f64 {
        match side {
            Side::Buy => MID - 1.0 - level as f64,
            Side::Sell => MID + level as f64,
        }
    }

    fn to_lob_side(side: Side) -> OrderSide {
        match side {
            Side::Buy => OrderSide::Buy,
            Side::Sell => OrderSide::Sell,
        }
    }

    fn oid(side: Side, level: i64) -> Oid {
        // Disjoint id space per side so bid/ask levels never collide.
        let base = match side {
            Side::Buy => 0u64,
            Side::Sell => 1u64 << 32,
        };
        Oid::new(base + level as u64)
    }

    pub struct LobBook {
        book: OrderBook,
        // lob's cancel/insert take an Oid, not a price — track which
        // level currently has a live order so insert-into-empty-level
        // stays a fair "insert", not a silent no-op re-insert.
        live: HashMap<(u8, i64), ()>,
        seq: u64,
    }

    impl LobBook {
        pub fn new() -> Self {
            LobBook { book: OrderBook::default(), live: HashMap::new(), seq: 0 }
        }

        fn key(side: Side, level: i64) -> (u8, i64) {
            (side as u8, level)
        }
    }

    impl BenchBook for LobBook {
        fn insert(&mut self, side: Side, level: i64, qty: i64) -> bool {
            self.seq += 1;
            let order = Order::new_limit(
                oid(side, level),
                to_lob_side(side),
                Timestamp::new(self.seq),
                price(side, level).into(),
                (qty as u64).into(),
            );
            if self.book.execute(&order).is_err() {
                return false;
            }
            self.live.insert(Self::key(side, level), ());
            true
        }

        /// Not exposed by lob 0.1.0 (no amend/modify-qty API on
        /// `OrderBook`) — unsupported by construction, not a bug.
        fn reduce(&mut self, _side: Side, _level: i64, _qty: i64) -> bool {
            false
        }

        fn cancel(&mut self, side: Side, level: i64) -> bool {
            let ok = self.book.cancel_order(oid(side, level)).is_ok();
            if ok {
                self.live.remove(&Self::key(side, level));
            }
            ok
        }

        /// lob's `OrderBook` keeps best-limit tracking internally
        /// (`Limits::get_best_limit`) but never exposes it publicly on
        /// `OrderBook` itself — unsupported by construction.
        fn best(&self, _side: Side) -> Option<i64> {
            None
        }

        fn match_touch(&mut self, side: Side, qty: i64) -> Option<i64> {
            self.seq += 1;
            let taker = Order::new_market(
                Oid::new(u64::MAX - self.seq),
                to_lob_side(side),
                Timestamp::new(self.seq),
                (qty as u64).into(),
            );
            // lob's `Trade` has no public filled-qty accessor (private
            // fields, Debug-only) — we can time the call but can't read
            // the fill amount back out of the crate's own return type.
            match self.book.execute(&taker) {
                Ok(_trade) => Some(0),
                Err(_) => None,
            }
        }

        fn supports_reduce(&self) -> bool {
            false
        }

        fn supports_best(&self) -> bool {
            false
        }

        fn supports_match(&self) -> bool {
            true
        }
    }
}

// NOTE: crates.io "orderbook" (inv2004, v0.1.9) was implemented and
// integrated here too, but is NOT wired into `all_contenders()`: it
// panics with an internal usize underflow (`index out of bounds`) the
// moment one side of the book empties completely and then refills —
// exactly the insert->cancel->insert cycle every op stream in this file
// drives, and an entirely realistic book event (a thin symbol going
// flat on one side). This is an upstream crash, not a dodge of an
// unfair test. See compare/orderbook-inv2004.md for the repro.

// ── Op-stream generators (shared, deterministic, net-neutral) ──────────

#[derive(Clone, Copy)]
enum Op {
    Insert { side: Side, level: i64, qty: i64 },
    Reduce { side: Side, level: i64, qty: i64 },
    Cancel { side: Side, level: i64 },
}

const LEVEL_COUNTS: [i64; 3] = [20, 100, 1000];
const BASE_QTY: i64 = 1000;
const REDUCE_STEPS: i64 = 3;

/// insert -> reduce (x REDUCE_STEPS, never to 0) -> cancel, one level
/// fully cycled before moving to the next. Every level starts AND ends
/// empty, so the whole stream can be cycled indefinitely by any
/// contender without violating "insert only targets an empty level".
fn gen_full_cycle_ops(m: i64) -> Vec<Op> {
    let mut ops = Vec::new();
    let step = BASE_QTY / (REDUCE_STEPS + 1);
    for level in 0..m {
        for side in [Side::Buy, Side::Sell] {
            ops.push(Op::Insert { side, level, qty: BASE_QTY });
            for k in 1..=REDUCE_STEPS {
                ops.push(Op::Reduce { side, level, qty: BASE_QTY - k * step });
            }
            ops.push(Op::Cancel { side, level });
        }
    }
    ops
}

/// insert -> cancel only (no reduce), for contenders that don't expose
/// a qty-reduce API (e.g. `lob`).
fn gen_insert_cancel_ops(m: i64) -> Vec<Op> {
    let mut ops = Vec::new();
    for level in 0..m {
        for side in [Side::Buy, Side::Sell] {
            ops.push(Op::Insert { side, level, qty: BASE_QTY });
            ops.push(Op::Cancel { side, level });
        }
    }
    ops
}

fn apply_op(book: &mut dyn BenchBook, op: Op) {
    match op {
        Op::Insert { side, level, qty } => {
            black_box(book.insert(side, level, qty));
        }
        Op::Reduce { side, level, qty } => {
            black_box(book.reduce(side, level, qty));
        }
        Op::Cancel { side, level } => {
            black_box(book.cancel(side, level));
        }
    }
}

/// Cycle `ops` for exactly `iters` applications (no cap-and-underreport:
/// unlike book_bench.rs's `iters.min(N)` pattern, this always measures
/// precisely `iters` ops, so it stays correct even when Criterion's
/// auto-selected `iters` runs past one pass through `ops`).
fn run_cycled(book: &mut dyn BenchBook, ops: &[Op], iters: u64) -> std::time::Duration {
    let mut idx = 0usize;
    let start = std::time::Instant::now();
    for _ in 0..iters {
        apply_op(book, ops[idx]);
        idx += 1;
        if idx >= ops.len() {
            idx = 0;
        }
    }
    start.elapsed()
}

// ── Harness ──────────────────────────────────────────────────────────

fn bench_level_touch_full(c: &mut Criterion) {
    let mut g = c.benchmark_group("level_touch_full");
    for &m in &LEVEL_COUNTS {
        let ops = gen_full_cycle_ops(m);
        for (name, factory) in all_contenders() {
            let probe = factory();
            if !probe.supports_reduce() {
                eprintln!("level_touch_full/{name}/{m}: N/A (no reduce API)");
                continue;
            }
            let ops = ops.clone();
            g.bench_with_input(BenchmarkId::new(name, m), &m, move |b, _| {
                b.iter_custom(|iters| {
                    let mut book = factory();
                    run_cycled(book.as_mut(), &ops, iters)
                });
            });
        }
    }
    g.finish();
}

fn bench_level_insert_cancel(c: &mut Criterion) {
    let mut g = c.benchmark_group("level_insert_cancel");
    for &m in &LEVEL_COUNTS {
        let ops = gen_insert_cancel_ops(m);
        for (name, factory) in all_contenders() {
            let ops = ops.clone();
            g.bench_with_input(BenchmarkId::new(name, m), &m, move |b, _| {
                b.iter_custom(|iters| {
                    let mut book = factory();
                    run_cycled(book.as_mut(), &ops, iters)
                });
            });
        }
    }
    g.finish();
}

fn bench_best_read(c: &mut Criterion) {
    let mut g = c.benchmark_group("best_read");
    for (name, factory) in all_contenders() {
        let mut probe = factory();
        if !probe.supports_best() {
            eprintln!("best_read/{name}: N/A (no public best-price API)");
            continue;
        }
        probe.insert(Side::Buy, 0, BASE_QTY);
        g.bench_function(name, move |b| {
            b.iter_custom(|iters| {
                let mut book = factory();
                book.insert(Side::Buy, 0, BASE_QTY);
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    black_box(book.best(Side::Buy));
                }
                start.elapsed()
            });
        });
    }
    g.finish();
}

fn bench_match_touch(c: &mut Criterion) {
    let mut g = c.benchmark_group("match_touch");
    for (name, factory) in all_contenders() {
        let probe = factory();
        if !probe.supports_match() {
            eprintln!(
                "match_touch/{name}: N/A (no order-level FIFO matching \
                 algorithm — L2-aggregated depth or feed-recording book)"
            );
            continue;
        }
        g.bench_function(name, move |b| {
            b.iter_custom(|iters| {
                let mut book = factory();
                // Deep enough resting liquidity that `iters` 1-lot
                // taker fills never exhaust the level (untimed setup).
                book.insert(Side::Sell, 0, iters as i64 + 10);
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    black_box(book.match_touch(Side::Buy, 1));
                }
                start.elapsed()
            });
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_level_touch_full,
    bench_level_insert_cancel,
    bench_best_read,
    bench_match_touch
);
criterion_main!(benches);
