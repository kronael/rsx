//! Tail-latency (p50/p99/p99.9/max) harness for the ME tile: the order
//! path on a warmed tile and the checkpoint capture pause on the pinned loop.
//!
//! NOT Criterion: a plain `fn main` wired as a `[[bench]] harness = false`
//! target, so one run yields every percentile of the same samples:
//!   `cargo bench -p rsx-matching --bench me_tail_bench`
//!
//! ## Order path
//!
//! `process_order` is the production function: this file includes
//! `src/order.rs` by path because the binary does not export it. Under
//! `cfg(test)` (clippy `--all-targets`) that file's test module needs
//! `cancel.rs` and `tile_test_utils.rs`, so they are included too. Each
//! order runs dedup, the ACCEPTED record, `process_new_order`,
//! `publish_events` to the WAL buffer and both cast senders, the order-index
//! update and the recenter check on the `tests/zero_heap_test.rs` tile: real
//! book, `WalWriter` in a temp dir, dedup, index, and two `CastSender`s aimed
//! at loopback sockets. Each order is timed on its own (an `Instant` pair,
//! ~25 ns, under 0.2% of an op that sends several UDP datagrams), so p99 is
//! a per-order quantile, not a batch average.
//!
//! The pinned loop also flushes the WAL every 10 ms and the peers (other
//! processes) drain their sockets. Both run here between timed orders: the
//! sockets are drained every `DRAIN_EVERY` orders, the WAL flushed every
//! `FLUSH_EVERY`. Neither is inside a sample.
//!
//! Rows, per resting depth per side:
//! - `accept_cross`: IOC buy, one non-draining partial fill (ACCEPTED, FILL,
//!   taker ORDER_DONE, BBO).
//! - `accept_rest`: GTC sell inside the spread (ACCEPTED, INSERTED, BBO).
//! - `accept_sweep`: IOC buy taking that resting order entirely (ACCEPTED,
//!   FILL, two ORDER_DONE, BBO).
//!
//! ## Checkpoint capture
//!
//! `MatchingCheckpointV1::capture` is the only checkpoint work on the pinned
//! loop (`cast_loop.rs`, every 10 s); encode and fsync run on the writer
//! thread. One sample is one capture call; dropping the snapshot is outside
//! the sample, because the writer thread drops it. `capture/<n>` books hold
//! `n` live orders; `capture/<n>_slots_10pct_live` is the largest book with
//! nine orders in ten cancelled, because `capture` sizes its visited vector by
//! the slab high-water mark, not by the live order count.
//!
//! ## Host noise
//!
//! Each row prints the involuntary context switches of the bench thread
//! (`nivcsw`) over the row. `accept_rest` and `accept_sweep` share one
//! phase, so they print the same count. A p99.9 or max near a scheduler
//! time slice with a non-zero `nivcsw` is the host preempting the bench.
//!
//! ## Gate
//!
//! p50 and p99 of every row are written to `target/tail/rsx-matching.json`
//! as `{name: ns}`; `scripts/bench-gate.sh` compares them with
//! `bench-baseline.json` like the Criterion estimates.
//!
//! Run on a quiet box with the RSX cluster stopped. On a shared host record
//! `nproc`, load average and `/proc/pressure/cpu` next to the numbers.

#[cfg(test)]
#[allow(dead_code, unused_imports)]
#[path = "../src/cancel.rs"]
mod cancel;
#[path = "harness.rs"]
mod harness;
#[allow(dead_code, unused_imports)]
#[path = "../src/order.rs"]
mod order;
#[cfg(test)]
#[allow(dead_code, unused_imports)]
#[path = "../src/tile_test_utils.rs"]
mod tile_test_utils;

use harness::MID;
use harness::SYMBOL_ID;
use order::process_order;
use rsx_book::book::Orderbook;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalWriter;
use rsx_matching::checkpoint::MatchingCheckpointV1;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::metrics::MatchingMetrics;
use rsx_matching::wal::CastPeer;
use rsx_matching::wal::OrderKey;
use rsx_messages::OrderMessage;
use rsx_types::Side;
use rustc_hash::FxHashMap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Instant;
use tempfile::TempDir;

const DEPTHS: [u64; 2] = [1_000, 10_000];
const WARMUP_ORDERS: usize = 20_000;
const ORDER_SAMPLES: usize = 100_000;
const DRAIN_EVERY: u32 = 32;
const FLUSH_EVERY: u32 = 1024;
const FLOOR_SAMPLES: usize = 100_000;

/// Resting orders in each capture book and the captures timed of it.
const CAPTURE_ROWS: [(u64, usize); 4] = [
    (1_000, 3_000),
    (10_000, 2_000),
    (100_000, 1_000),
    (1_000_000, 500),
];
const ORDERS_PER_LEVEL: u64 = 4;
const CAPTURE_MID: i64 = 1_000_000;

const BUY: u8 = 0;
const SELL: u8 = 1;
const GTC: u8 = 0;
const IOC: u8 = 1;
const GATEWAY_IN_NS: u64 = 1_700_000_000_000_000_000;

struct Percentiles {
    p50: f64,
    p99: f64,
    p999: f64,
    max: f64,
    mean: f64,
    samples: usize,
}

/// Nearest-rank quantiles of `samples_ns` (no interpolation).
fn percentiles(mut samples_ns: Vec<f64>) -> Percentiles {
    samples_ns.sort_by(|a, b| a.partial_cmp(b).expect("INVARIANT: no NaN timings"));
    let n = samples_ns.len();
    let at = |q: f64| samples_ns[((n as f64 * q) as usize).min(n - 1)];
    Percentiles {
        p50: at(0.50),
        p99: at(0.99),
        p999: at(0.999),
        max: samples_ns[n - 1],
        mean: samples_ns.iter().sum::<f64>() / n as f64,
        samples: n,
    }
}

fn involuntary_switches() -> i64 {
    // SAFETY: getrusage writes the zeroed rusage it is handed.
    unsafe {
        let mut usage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_THREAD, &mut usage);
        usage.ru_nivcsw
    }
}

#[derive(Default)]
struct Rows {
    gate: Vec<(String, f64)>,
}

impl Rows {
    fn record(&mut self, name: &str, stats: &Percentiles, switches: i64) {
        println!(
            "{name:<34} n={:<7} mean={:>11.0}ns p50={:>11.0}ns p99={:>11.0}ns \
             p99.9={:>11.0}ns max={:>12.0}ns nivcsw={switches}",
            stats.samples, stats.mean, stats.p50, stats.p99, stats.p999, stats.max
        );
        self.gate.push((format!("me_tail/{name}/p50"), stats.p50));
        self.gate.push((format!("me_tail/{name}/p99"), stats.p99));
    }

    fn write_gate_file(&self) {
        let dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"))
            .join("tail");
        std::fs::create_dir_all(&dir).expect("create target/tail");
        let mut json = String::from("{\n");
        for (index, (name, ns)) in self.gate.iter().enumerate() {
            let comma = if index + 1 == self.gate.len() {
                ""
            } else {
                ","
            };
            writeln!(json, "  \"{name}\": {ns:.1}{comma}").expect("write to String");
        }
        json.push_str("}\n");
        let path = dir.join("rsx-matching.json");
        std::fs::write(&path, json).expect("write gate file");
        println!("\ngate entries: {}", path.display());
    }
}

fn rest(book: &mut Orderbook, side: Side, price: i64, qty: i64, oid: u64) -> u32 {
    book.insert_resting(price, qty, side, 0, 200, false, 1, 0, oid)
}

/// `depth` resting orders per side, one per tick; both touches hold
/// `BIG_QTY`, so a qty-1 taker never drains the ask.
fn build_tile_book(depth: u64) -> Orderbook {
    let mut book = Orderbook::new(harness::config(), (2 * depth + 4_096) as u32, MID);
    rest(&mut book, Side::Sell, MID + 1, harness::BIG_QTY, 1);
    rest(&mut book, Side::Buy, MID - 1, harness::BIG_QTY, 2);
    for i in 2..=depth as i64 {
        rest(&mut book, Side::Sell, MID + i, 10, 100 + i as u64);
        rest(&mut book, Side::Buy, MID - i, 10, 1_000_000 + i as u64);
    }
    book
}

struct Tile {
    book: Orderbook,
    wal_writer: WalWriter,
    risk: CastPeer,
    mkt: CastPeer,
    index: FxHashMap<OrderKey, u32>,
    dedup: DedupTracker,
    metrics: MatchingMetrics,
    sockets: [UdpSocket; 2],
    next_oid: u64,
    since_drain: u32,
    since_flush: u32,
    _wal_dir: TempDir,
}

impl Tile {
    fn new(depth: u64) -> Self {
        let wal_dir = TempDir::new().expect("tempdir");
        let sockets = [
            UdpSocket::bind("127.0.0.1:0").expect("bind risk peer"),
            UdpSocket::bind("127.0.0.1:0").expect("bind marketdata peer"),
        ];
        for socket in &sockets {
            socket.set_nonblocking(true).expect("nonblocking");
        }
        let peer = |name, socket: &UdpSocket| {
            CastPeer::new(
                name,
                CastSender::new(socket.local_addr().unwrap(), SYMBOL_ID, wal_dir.path()).unwrap(),
            )
        };
        Self {
            book: build_tile_book(depth),
            wal_writer: WalWriter::new(SYMBOL_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap(),
            risk: peer("risk", &sockets[0]),
            mkt: peer("marketdata", &sockets[1]),
            index: FxHashMap::default(),
            dedup: DedupTracker::new(),
            metrics: MatchingMetrics::default(),
            sockets,
            next_oid: 10_000_000,
            since_drain: 0,
            since_flush: 0,
            _wal_dir: wal_dir,
        }
    }

    /// Runs one order through `process_order`; returns its nanoseconds.
    fn timed(&mut self, user_id: u32, side: u8, tif: u8, price: i64, qty: i64) -> f64 {
        self.next_oid += 1;
        let message = OrderMessage {
            seq: 0,
            price,
            qty,
            side,
            tif,
            reduce_only: 0,
            post_only: 0,
            _pad1: [0; 4],
            user_id,
            _pad2: 0,
            timestamp_ns: GATEWAY_IN_NS,
            order_id_hi: 0,
            order_id_lo: self.next_oid,
            risk_in_ns: GATEWAY_IN_NS,
            _pad3: [0; 56],
        };
        let start = Instant::now();
        process_order(
            &mut self.book,
            &mut self.wal_writer,
            &mut self.risk,
            &mut self.mkt,
            &mut self.index,
            &mut self.dedup,
            &self.metrics,
            SYMBOL_ID,
            black_box(&message),
        );
        let ns = start.elapsed().as_nanos() as f64;
        self.between_orders();
        ns
    }

    fn between_orders(&mut self) {
        self.since_drain += 1;
        self.since_flush += 1;
        if self.since_drain == DRAIN_EVERY {
            self.since_drain = 0;
            let mut buf = [0u8; 2048];
            for socket in &self.sockets {
                while socket.recv(&mut buf).is_ok() {}
            }
        }
        if self.since_flush == FLUSH_EVERY {
            self.since_flush = 0;
            self.wal_writer.flush().expect("wal flush");
        }
    }
}

fn bench_order_path(rows: &mut Rows) {
    for &depth in &DEPTHS {
        let mut tile = Tile::new(depth);

        for _ in 0..WARMUP_ORDERS {
            tile.timed(2, BUY, IOC, MID + 1, 1);
        }
        let switches = involuntary_switches();
        let samples = (0..ORDER_SAMPLES)
            .map(|_| tile.timed(2, BUY, IOC, MID + 1, 1))
            .collect();
        rows.record(
            &format!("accept_cross/{depth}"),
            &percentiles(samples),
            involuntary_switches() - switches,
        );

        for _ in 0..WARMUP_ORDERS / 2 {
            tile.timed(1, SELL, GTC, MID, 10);
            tile.timed(2, BUY, IOC, MID, 10);
        }
        let switches = involuntary_switches();
        let mut rests = Vec::with_capacity(ORDER_SAMPLES);
        let mut sweeps = Vec::with_capacity(ORDER_SAMPLES);
        for _ in 0..ORDER_SAMPLES {
            rests.push(tile.timed(1, SELL, GTC, MID, 10));
            sweeps.push(tile.timed(2, BUY, IOC, MID, 10));
        }
        let switches = involuntary_switches() - switches;
        rows.record(
            &format!("accept_rest/{depth}"),
            &percentiles(rests),
            switches,
        );
        rows.record(
            &format!("accept_sweep/{depth}"),
            &percentiles(sweeps),
            switches,
        );
        println!();
    }
}

/// `orders` resting orders, `ORDERS_PER_LEVEL` per price level, both sides;
/// returns the book and every order handle.
fn build_capture_book(orders: u64) -> (Orderbook, Vec<u32>) {
    let levels = orders / (2 * ORDERS_PER_LEVEL);
    let mut book = Orderbook::new(harness::config(), (orders + 1_024) as u32, CAPTURE_MID);
    let mut handles = Vec::with_capacity(orders as usize);
    let mut oid = 1;
    for level in 1..=levels as i64 {
        for _ in 0..ORDERS_PER_LEVEL {
            handles.push(rest(&mut book, Side::Sell, CAPTURE_MID + level, 10, oid));
            handles.push(rest(&mut book, Side::Buy, CAPTURE_MID - level, 10, oid + 1));
            oid += 2;
        }
    }
    (book, handles)
}

fn measure_capture(rows: &mut Rows, name: &str, book: &Orderbook, samples: usize) {
    for _ in 0..3 {
        black_box(MatchingCheckpointV1::capture(black_box(book), 42).expect("capture"));
    }
    let switches = involuntary_switches();
    let mut samples_ns = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let checkpoint = MatchingCheckpointV1::capture(black_box(book), 42).expect("capture");
        samples_ns.push(start.elapsed().as_nanos() as f64);
        drop(black_box(checkpoint));
    }
    rows.record(
        name,
        &percentiles(samples_ns),
        involuntary_switches() - switches,
    );
}

fn bench_capture(rows: &mut Rows) {
    for &(orders, samples) in &CAPTURE_ROWS {
        let (book, handles) = build_capture_book(orders);
        drop(handles);
        measure_capture(rows, &format!("capture/{orders}"), &book, samples);
    }

    let (orders, samples) = CAPTURE_ROWS[CAPTURE_ROWS.len() - 1];
    let (mut book, handles) = build_capture_book(orders);
    for (index, &handle) in handles.iter().enumerate() {
        if index % 10 != 0 {
            assert!(book.cancel_order(handle));
        }
    }
    drop(handles);
    measure_capture(
        rows,
        &format!("capture/{orders}_slots_10pct_live"),
        &book,
        samples,
    );
}

fn main() {
    harness::pin();
    let mut rows = Rows::default();

    let floor = percentiles(
        (0..FLOOR_SAMPLES)
            .map(|_| {
                let start = Instant::now();
                black_box(Instant::now()).duration_since(start).as_nanos() as f64
            })
            .collect(),
    );
    println!("timer floor: back-to-back Instant::now(), n={FLOOR_SAMPLES}");
    println!(
        "timer_floor p50={:.0}ns p99={:.0}ns\n",
        floor.p50, floor.p99
    );

    bench_order_path(&mut rows);
    bench_capture(&mut rows);
    rows.write_gate_file();
}
