//! Tail-latency (p50/p99/p99.9) harness for rsx-book's hot ops.
//!
//! Criterion (the rest of this bench suite) only reports a median across
//! `sample_size` batches — it never surfaces the tail. The CEO-eval gap
//! this closes: "all figures are p50; a serious bet wants the tail,
//! especially on the just-fixed level-clearing path" (the occupancy-
//! bitmap `match_clears` op, see `reports/20260704_book-bench.md`).
//!
//! NOT Criterion: a plain `fn main`, wired as a `[[bench]] harness =
//! false` target so it runs standalone:
//!   `cargo bench -p rsx-book --bench tail_bench`
//! (or `cargo run --release ...` gives the same binary; `bench` keeps it
//! grouped with the rest of the suite in one command).
//!
//! ## Methodology — the timer floor is the whole story at ~60-150ns
//!
//! These ops run in tens of nanoseconds. `Instant::now()` on Linux reads
//! a vDSO `clock_gettime(CLOCK_MONOTONIC)` — typically ~20-40 ns of
//! overhead, a LARGE fraction of the op. A naive "time every single op"
//! loop mostly measures timer noise, not the op, and its p99 would be a
//! lie. This harness measures two things and reports both, honestly:
//!
//! 1. **Batch-amortized (the defensible number).** Time a batch of
//!    `BATCH` consecutive ops as one span, divide by `BATCH`. Timer
//!    overhead is paid once per batch, so at `BATCH=64` it contributes
//!    well under 1 ns to the reported per-op figure. The batch-time
//!    DISTRIBUTION (sorted, percentiled) is what reveals a tail: a rare
//!    slow op (branch misprediction on a bitmap word-boundary crossing,
//!    a cache miss, scheduler jitter) shows up as an elevated batch
//!    average even after dividing by 64. This is the number quoted in
//!    the report.
//! 2. **Single-op raw timer samples (context only).** Also reported,
//!    alongside the measured **timer floor** (back-to-back `Instant::
//!    now()` calls with no op in between). When the op's single-op p50
//!    is within noise of the floor, that is stated explicitly — it means
//!    the single-op *distribution* is timer noise, not op behavior, and
//!    only the batch-amortized number should be trusted.
//!
//! Every op result goes through `black_box` so the compiler cannot elide
//! it. `WARMUP` iterations run and are discarded before any measurement
//! (cache/branch-predictor warmup). Sample sizes are large enough
//! (`BATCH_SAMPLES` ~31k batch samples, `SINGLE_SAMPLES` 100k single-op
//! samples) that p99.9 is a real quantile, not an artifact of a handful
//! of points.
//!
//! Run: `cargo bench -p rsx-book --bench tail_bench` on a quiet box
//! (stop the RSX cluster first — see `reports/20260704_book-bench.md`).

#[path = "harness.rs"]
mod harness;

use harness::MID;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::TimeInForce;
use std::hint::black_box;
use std::time::Instant;

/// Resting depth per side behind the touch (excluding the touch itself).
const DEPTHS: [u64; 2] = [1_000, 10_000];

/// Discarded iterations before any measurement (cache/branch warmup).
const WARMUP: usize = 20_000;

/// Ops per batch for the amortized measurement. At 64, a ~30 ns timer
/// overhead contributes <0.5 ns to the reported per-op figure.
const BATCH: usize = 64;

/// Total ops fed through the batch-amortized measurement (per op/depth).
/// 2,000,000 / BATCH = 31,250 batch samples -> ~31 points above p99.9.
const BATCH_TOTAL_OPS: usize = 2_000_000;

/// Sample count for the single-op raw-timer distribution (context only).
const SINGLE_SAMPLES: usize = 100_000;

/// Sample count for the back-to-back-timer-call floor measurement.
const FLOOR_SAMPLES: usize = 100_000;

/// A far, always-empty ask price used by cancel_deep (never populated by
/// any dense-shape build below, so it is a fresh slot every call).
const DEEP_ASK: i64 = MID + 2_000_000;

struct Percentiles {
    p50: f64,
    p99: f64,
    p999: f64,
    max: f64,
    mean: f64,
}

/// Sort `samples_ns` and pull out the percentiles used in the report.
/// Nearest-rank (no interpolation) — simplest defensible method at
/// these sample sizes.
fn percentiles(mut samples_ns: Vec<f64>) -> Percentiles {
    samples_ns.sort_by(|a, b| {
        a.partial_cmp(b).expect("INVARIANT: no NaN timings")
    });
    let n = samples_ns.len();
    let at = |q: f64| samples_ns[((n as f64 * q) as usize).min(n - 1)];
    let mean = samples_ns.iter().sum::<f64>() / n as f64;
    Percentiles {
        p50: at(0.50),
        p99: at(0.99),
        p999: at(0.999),
        max: samples_ns[n - 1],
        mean,
    }
}

/// Time `BATCH_TOTAL_OPS / BATCH` batches of `BATCH` consecutive `op()`
/// calls; each batch contributes one (elapsed / BATCH) sample.
fn measure_batch(mut op: impl FnMut()) -> Percentiles {
    let n_batches = BATCH_TOTAL_OPS / BATCH;
    let mut samples = Vec::with_capacity(n_batches);
    for _ in 0..n_batches {
        let start = Instant::now();
        for _ in 0..BATCH {
            op();
        }
        let elapsed = start.elapsed();
        samples.push(elapsed.as_nanos() as f64 / BATCH as f64);
    }
    percentiles(samples)
}

/// Time each `op()` call individually — dominated by timer overhead at
/// these op sizes; reported alongside the floor for context, never
/// quoted as the headline number.
fn measure_single(mut op: impl FnMut()) -> Percentiles {
    let mut samples = Vec::with_capacity(SINGLE_SAMPLES);
    for _ in 0..SINGLE_SAMPLES {
        let start = Instant::now();
        op();
        samples.push(start.elapsed().as_nanos() as f64);
    }
    percentiles(samples)
}

/// Back-to-back `Instant::now()` calls, nothing timed in between: the
/// timer's own noise floor, measured the same way as `measure_single`.
fn measure_floor() -> Percentiles {
    let mut samples = Vec::with_capacity(FLOOR_SAMPLES);
    for _ in 0..FLOOR_SAMPLES {
        let start = Instant::now();
        let mid = black_box(Instant::now());
        samples.push(mid.duration_since(start).as_nanos() as f64);
    }
    percentiles(samples)
}

fn print_row(name: &str, kind: &str, p: &Percentiles) {
    println!(
        "{name:<24} {kind:<10} mean={:>7.1}ns p50={:>7.1}ns \
         p99={:>7.1}ns p99.9={:>7.1}ns max={:>9.1}ns",
        p.mean, p.p50, p.p99, p.p999, p.max
    );
}

fn rest(
    book: &mut Orderbook,
    buy: bool,
    price: i64,
    qty: i64,
    oid: u64,
) -> u32 {
    let side = if buy { Side::Buy } else { Side::Sell };
    book.insert_resting(price, qty, side, 0, 1, false, 1, 0, oid)
}

/// A book with a 1-order touch at MID+1 / MID-1 (given `touch_qty`) and
/// `n` contiguous resting orders per side behind it (dense shape, same
/// layout as `distribution_bench`'s `Shape::Dense`).
fn build_dense(n: u64, touch_qty: i64) -> Orderbook {
    let cap = (2 * n + 8_192) as u32;
    let mut book = Orderbook::new(harness::config(), cap, MID);
    rest(&mut book, false, MID + 1, touch_qty, 1);
    rest(&mut book, true, MID - 1, touch_qty, 2);
    let mut oid = 100u64;
    for i in 2..=(n as i64 + 1) {
        rest(&mut book, false, MID + i, 10, oid);
        oid += 1;
        rest(&mut book, true, MID - i, 10, oid);
        oid += 1;
    }
    book
}

fn taker_ioc_buy(book: &mut Orderbook, price: i64, qty: i64, oid: u64) {
    let mut o = IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side: Side::Buy,
        tif: TimeInForce::IOC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut o);
    black_box(&o);
}

/// Run both measurements for one op/depth combo and print the rows.
fn bench_op(label: &str, mut op: impl FnMut()) {
    for _ in 0..WARMUP {
        op();
    }
    let batch = measure_batch(&mut op);
    print_row(label, "batch/64", &batch);
    let single = measure_single(&mut op);
    print_row(label, "single", &single);
}

fn main() {
    harness::pin();

    println!(
        "timer floor: back-to-back Instant::now(), n={FLOOR_SAMPLES}"
    );
    let floor = measure_floor();
    print_row("timer_floor", "single", &floor);
    println!();

    for &depth in &DEPTHS {
        // match: partial fill, touch survives (the happy path). Touch
        // qty is huge relative to BATCH_TOTAL_OPS + WARMUP so it never
        // empties across the whole run — every fill is a partial fill.
        {
            let mut book = build_dense(depth, 10_000_000_000);
            let mut oid = 10_000_000u64;
            bench_op(&format!("match/{depth}"), move || {
                taker_ioc_buy(&mut book, MID + 1, 1, oid);
                oid += 1;
            });
        }

        // match_clears: IOC taker consumes the 1-order touch (occupancy
        // scan fires to find next-best), then replenish. THE path that
        // carried the level-clearing fix — the tail that matters most.
        {
            let mut book = build_dense(depth, 10);
            let mut oid = 20_000_000u64;
            bench_op(&format!("match_clears/{depth}"), move || {
                taker_ioc_buy(&mut book, MID + 1, 10, oid);
                oid += 1;
                rest(&mut book, false, MID + 1, 10, oid);
                oid += 1;
            });
        }

        // cancel_touch: insert a new sole-best ask, cancel it (empties
        // -> best recompute via occupancy scan). Net-neutral.
        {
            let mut book = build_dense(depth, 10);
            let mut oid = 30_000_000u64;
            bench_op(&format!("cancel_touch/{depth}"), move || {
                let h = rest(&mut book, false, MID, 1, oid);
                oid += 1;
                black_box(book.cancel_order(h));
            });
        }

        // cancel_deep: insert+cancel an isolated far level (never the
        // best, so no scan) — the distribution-independent baseline.
        {
            let mut book = build_dense(depth, 10);
            let mut oid = 40_000_000u64;
            bench_op(&format!("cancel_deep/{depth}"), move || {
                let h = rest(&mut book, false, DEEP_ASK, 1, oid);
                oid += 1;
                black_box(book.cancel_order(h));
            });
        }
        println!();
    }
}
