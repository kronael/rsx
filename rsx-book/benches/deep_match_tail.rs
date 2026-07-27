//! Fixed-work partial-fill distribution at deep resting depths.
//!
//! Each sample is a 64-operation batch average. The maker at the touch
//! never drains, so every timed operation does the same work. The table
//! reports p50, bootstrap standard error of p50, and p99 from one run.

#[path = "harness.rs"]
mod harness;

use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::TimeInForce;
use std::hint::black_box;
use std::time::Instant;

const DEPTHS: [u64; 3] = [100_000, 1_000_000, 10_000_000];
const WARMUP: usize = 20_000;
const BATCH: usize = 64;
const SAMPLES: usize = 31_250;
const BOOTSTRAPS: usize = 500;
const TOUCH_QTY: i64 = 10_000_000_000;

struct Estimates {
    p50: f64,
    p50_se: f64,
    p99: f64,
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn taker_fill(book: &mut rsx_book::Orderbook, order_id: u64) {
    let mut order = IncomingOrder {
        price: harness::MID + 1,
        qty: 1,
        remaining_qty: 1,
        side: Side::Buy,
        tif: TimeInForce::IOC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo: order_id,
    };
    process_new_order(black_box(book), black_box(&mut order));
}

fn median(values: &mut [f64]) -> f64 {
    let middle = values.len() / 2;
    values.select_nth_unstable_by(middle, |a, b| {
        a.partial_cmp(b).expect("timings must not contain NaN")
    });
    values[middle]
}

fn median_standard_error(samples: &[f64], seed: u64) -> f64 {
    let mut rng = Rng::new(seed);
    let mut resample = vec![0.0; samples.len()];
    let mut medians = Vec::with_capacity(BOOTSTRAPS);
    for _ in 0..BOOTSTRAPS {
        for value in &mut resample {
            *value = samples[rng.next() as usize % samples.len()];
        }
        medians.push(median(&mut resample));
    }
    let mean = medians.iter().sum::<f64>() / medians.len() as f64;
    let variance = medians
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (medians.len() - 1) as f64;
    variance.sqrt()
}

fn estimates(mut samples: Vec<f64>, seed: u64) -> Estimates {
    let p50_se = median_standard_error(&samples, seed);
    samples.sort_by(|a, b| a.partial_cmp(b).expect("timings must not contain NaN"));
    Estimates {
        p50: samples[samples.len() / 2],
        p50_se,
        p99: samples[(samples.len() * 99) / 100],
    }
}

fn measure(depth: u64) -> Estimates {
    let mut book = harness::build(depth - 1);
    book.insert_resting(
        harness::MID + 1,
        TOUCH_QTY,
        Side::Sell,
        0,
        1,
        false,
        1,
        0,
        1,
    );
    let mut order_id = 2;
    for _ in 0..WARMUP {
        taker_fill(&mut book, order_id);
        order_id += 1;
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..BATCH {
            taker_fill(&mut book, order_id);
            order_id += 1;
        }
        samples.push(start.elapsed().as_nanos() as f64 / BATCH as f64);
    }
    estimates(samples, depth ^ 0x5eed_f00d)
}

fn main() {
    harness::pin();
    println!("resting orders | p50 ± SE | p99");
    for depth in DEPTHS {
        let result = measure(depth);
        println!(
            "{depth:>14} | {:>7.3} ± {:>5.3} ns | {:>7.3} ns",
            result.p50, result.p50_se, result.p99
        );
    }
}
