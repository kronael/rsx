//! Deep-book benchmark: prove insert + match latency stays flat as the
//! resting book grows to 100k / 1m / 10m orders.
//!
//! Book model (verified against rsx-book):
//! - `insert_resting` rests directly (no match path) and emits no events,
//!   so seeding cannot cross-match — the book deterministically reaches
//!   exactly N resting orders. Seeding is reproducible (fixed PRNG seed).
//! - `process_new_order` resets the event buffer per call, so matching
//!   never accumulates events across iterations.
//!
//! Distributions:
//! - price offset ~ Student-t(NU) * SCALE — heavy-tailed (fat tails, like
//!   real markets), assigned to bid/ask side with a fixed half-spread so
//!   the seed is never crossed (bids < mid-HS, asks > mid+HS). Offset is
//!   clamped to MAX_OFF so extreme tail draws stay inside the price range.
//! - resting size ~ BASE * (1 + K*dist/SCALE) * Unif(0.5,1.5): grows with
//!   distance from the mean (deeper levels carry more size).
//! - taker (fill) size ~ BASE * (1 + Exp(mean=1)), side ~ Bernoulli(0.5):
//!   simple, occasionally sweeps a few levels.
//!
//! Each measured op is net-neutral on book size (insert+cancel; match+
//! replenish), so depth stays ~N across criterion's iterations — the
//! latency we report is genuinely "at depth N", not a depleting book.
//!
//! Run: cargo bench -p rsx-book --bench deep_book_bench

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

const MID: i64 = 1_000_000;
const SCALE: f64 = 12_000.0; // Student-t scale (body ~ prior normal sigma)
const NU: u32 = 3; // t degrees of freedom: low = heavy tails
const MAX_OFF: i64 = 450_000; // clamp extreme tail draws into price range
const HALF_SPREAD: i64 = 50; // guarantees bids < asks at seed
const BASE: f64 = 100.0; // base resting/taker size (lots)
const SIZE_K: f64 = 0.5; // resting size growth with distance from mean
const SIZES: [u64; 3] = [100_000, 1_000_000, 10_000_000];

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

/// xorshift64* — deterministic, self-contained (no rand dependency).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    fn unif(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Standard normal via Box-Muller.
    fn normal(&mut self) -> f64 {
        let u1 = self.unif().max(1e-12);
        let u2 = self.unif();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    /// Exponential with given mean.
    fn exp(&mut self, mean: f64) -> f64 {
        -mean * self.unif().max(1e-12).ln()
    }
    /// Student-t with `nu` degrees of freedom: t = Z / sqrt(chi2_nu / nu).
    /// Low nu => heavy tails (fat-tailed price spread, like real markets).
    fn student_t(&mut self, nu: u32) -> f64 {
        let z = self.normal();
        let mut chi2 = 0.0;
        for _ in 0..nu {
            let n = self.normal();
            chi2 += n * n;
        }
        z / (chi2 / nu as f64).sqrt()
    }
}

/// Draw one resting order: normal price around mid, side by coin flip,
/// size that grows with distance from the mean.
fn draw_resting(rng: &mut Rng) -> (i64, i64, Side) {
    let raw = (rng.student_t(NU).abs() * SCALE).round() as i64;
    let off = (HALF_SPREAD + raw).min(MAX_OFF);
    let buy = rng.next_u64() & 1 == 0;
    let price = if buy { MID - off } else { MID + off };
    let noise = 0.5 + rng.unif(); // [0.5, 1.5)
    let qty = (BASE * (1.0 + SIZE_K * off as f64 / SCALE) * noise)
        .round()
        .max(1.0) as i64;
    (price, qty, if buy { Side::Buy } else { Side::Sell })
}

/// Build a book seeded with exactly `n` resting orders. Deterministic:
/// the same `n` always produces the same book. Prints seed stats so the
/// caller can confirm seeding is stable across runs.
fn build(n: u64) -> Orderbook {
    let mut book = Orderbook::new(config(), (n + 1024) as u32, MID);
    let mut rng = Rng::new(0x1234_5678_9abc_def0 ^ n);
    let mut total_qty: i128 = 0;
    let mut min_p = i64::MAX;
    let mut max_p = i64::MIN;
    for i in 0..n {
        let (p, q, s) = draw_resting(&mut rng);
        book.insert_resting(p, q, s, 0, (i % 100_000) as u32 + 1, false, 1, 0, i + 1);
        total_qty += q as i128;
        min_p = min_p.min(p);
        max_p = max_p.max(p);
    }
    eprintln!(
        "seed n={n} total_qty={total_qty} price=[{min_p}..{max_p}] (deterministic)"
    );
    book
}

/// Insert at depth: insert one order then cancel it (net-neutral, so the
/// book stays at N). Measures insert+cancel against an N-deep book.
fn bench_insert(c: &mut Criterion) {
    let mut g = c.benchmark_group("deep_insert");
    g.sample_size(20);
    for &n in &SIZES {
        let mut book = build(n);
        let mut rng = Rng::new(0xBEEF ^ n);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let (p, q, s) = draw_resting(&mut rng);
                let h = book.insert_resting(p, q, s, 0, 1, false, 1, 0, 0);
                black_box(book.cancel_order(h));
            });
        });
    }
    g.finish();
}

/// Match at depth: a marketable IOC taker drawn from the fill distribution
/// consumes a few resting orders near the touch; replenish the consumed
/// quantity on the same side so depth stays ~N. Measures match+replenish.
fn bench_match(c: &mut Criterion) {
    let mut g = c.benchmark_group("deep_match");
    g.sample_size(20);
    for &n in &SIZES {
        let mut book = build(n);
        let mut rng = Rng::new(0xF00D ^ n);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let buy = rng.next_u64() & 1 == 0;
                let q = (BASE * (1.0 + rng.exp(1.0))).round().max(1.0) as i64;
                let side = if buy { Side::Buy } else { Side::Sell };
                // Marketable far past the touch so it sweeps from best.
                let px = if buy { MID + 1_000_000 } else { MID - 1_000_000 };
                let mut o = IncomingOrder {
                    price: px,
                    qty: q,
                    remaining_qty: q,
                    side,
                    tif: TimeInForce::IOC,
                    user_id: 1,
                    reduce_only: false,
                    post_only: false,
                    timestamp_ns: 1,
                    order_id_hi: 0,
                    order_id_lo: 0,
                };
                process_new_order(black_box(&mut book), black_box(&mut o));
                let consumed = q - o.remaining_qty;
                if consumed > 0 {
                    let (rp, rs) = if buy {
                        (MID + HALF_SPREAD, Side::Sell)
                    } else {
                        (MID - HALF_SPREAD, Side::Buy)
                    };
                    book.insert_resting(rp, consumed, rs, 0, 1, false, 1, 0, 0);
                }
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench_insert, bench_match);
criterion_main!(benches);
