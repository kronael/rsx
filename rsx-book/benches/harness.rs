//! Shared bench harness for rsx-book. Centralizes core pinning, the
//! Criterion config, the symbol config, and the book fixtures so every
//! rsx-book bench measures against identically-built books with the
//! same statistics. Included by each bench via
//! `#[path = "harness.rs"] mod harness;` — this file is NOT a bench
//! target itself (no `criterion_main`), so it has no Cargo.toml entry.
//!
//! Drift between benches is how unfair numbers creep in; keeping pin +
//! config + fixtures in one place is the point. Keep it minimal.
#![allow(dead_code)]

use core_affinity::CoreId;
use criterion::Criterion;
use rsx_book::book::Orderbook;
use rsx_types::Side;
use rsx_types::SymbolConfig;

/// Core the timed Criterion thread pins to. Matches the cast harness
/// convention (client -> core 2) so cross-crate runs use the same core.
pub const BENCH_CORE: usize = 2;

/// Book mid price for all fixtures (i64 fixed-point, 2 price decimals).
pub const MID: i64 = 1_000_000;

/// Pin the current (Criterion timer) thread to a fixed core. Safe to
/// call once at the top of every bench fn; falls back to core 0 if the
/// box has fewer cores.
pub fn pin() {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let core = ids.get(BENCH_CORE).copied().unwrap_or(CoreId { id: 0 });
    core_affinity::set_for_current(core);
}

/// The one shared Criterion config. `sample_size(50)` matches the cast
/// benches so cross-crate numbers use the same statistics.
pub fn criterion() -> Criterion {
    Criterion::default().sample_size(50)
}

/// Symbol config shared by every fixture: tick 1, lot 1 => raw units,
/// so bench prices/qtys are the fixed-point values directly.
pub fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

// --- Fat-tailed seeding (shared with the depth-curve benches) ---------
//
// Fixed constants so a given depth always produces the same book; the
// deep-book flat-latency numbers reproduce run to run.

/// Student-t scale (body ~ prior normal sigma).
pub const SCALE: f64 = 12_000.0;
/// t degrees of freedom: low = heavy tails (fat-tailed spread).
pub const NU: u32 = 3;
/// Clamp extreme tail draws into the compression price range.
pub const MAX_OFF: i64 = 450_000;
/// Guarantees seeded bids < asks (no crossed seed book).
pub const HALF_SPREAD: i64 = 50;
/// Base resting/taker size (lots).
pub const BASE: f64 = 100.0;
/// Resting size growth with distance from the mean.
pub const SIZE_K: f64 = 0.5;

/// xorshift64* — deterministic, self-contained (no rand dependency).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn unif(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Standard normal via Box-Muller.
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unif().max(1e-12);
        let u2 = self.unif();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    /// Exponential with given mean.
    pub fn exp(&mut self, mean: f64) -> f64 {
        -mean * self.unif().max(1e-12).ln()
    }
    /// Student-t with `nu` dof: t = Z / sqrt(chi2_nu / nu). Low nu =>
    /// heavy tails (fat-tailed price spread, like real markets).
    pub fn student_t(&mut self, nu: u32) -> f64 {
        let z = self.normal();
        let mut chi2 = 0.0;
        for _ in 0..nu {
            let n = self.normal();
            chi2 += n * n;
        }
        z / (chi2 / nu as f64).sqrt()
    }
}

/// Draw one resting order: fat-tailed price around mid, side by coin
/// flip, size that grows with distance from the mean.
pub fn draw_resting(rng: &mut Rng) -> (i64, i64, Side) {
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
/// the same `n` always produces the same book. `insert_resting` rests
/// directly (no match path, emits no events), so seeding never
/// cross-matches — the book reaches exactly `n` resting orders.
///
/// Slab headroom: the depth benches sit a churn pool ON TOP of the
/// n-deep book — cancel_group inserts up to 4096 cancelable handles
/// before freeing them, and match_group transiently allocates the
/// taker + replenish slots. Size for n + that peak (8192 covers the
/// 4096 pool plus match churn) so the slab never exhausts. Additive,
/// not multiplicative, so the 10M deep book stays RAM-bound at ~n.
pub fn build(n: u64) -> Orderbook {
    let mut book = Orderbook::new(config(), (n + 8192) as u32, MID);
    let mut rng = Rng::new(0x1234_5678_9abc_def0 ^ n);
    for i in 0..n {
        let (p, q, s) = draw_resting(&mut rng);
        book.insert_resting(p, q, s, 0, (i % 100_000) as u32 + 1, false, 1, 0, i + 1);
    }
    book
}
