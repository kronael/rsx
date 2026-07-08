//! Shared bench harness for rsx-matching. Centralizes core pinning, the
//! Criterion config, the symbol config, and the ME fixtures so every
//! rsx-matching bench measures identically-constructed state with the
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
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::update_order_index;
use rsx_matching::wal::write_events_to_wal;
use rsx_matching::wal::OrderKey;
use rsx_messages::OrderAcceptedRecord;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use tempfile::TempDir;

/// Core the timed Criterion thread pins to. Matches the cast + book
/// harness convention (client -> core 2) so cross-crate runs use the
/// same core.
pub const BENCH_CORE: usize = 2;

/// Book mid price for all fixtures (i64 fixed-point). Matches the value
/// the existing matching benches used so carried-over numbers line up.
pub const MID: i64 = 100_000;

/// Symbol id shared by every fixture.
pub const SYMBOL_ID: u32 = 1;

/// Resting-maker size big enough that a stream of qty-1 takers never
/// drains the best level, so book depth stays fixed across a whole
/// Criterion run (the match stays a single partial fill).
pub const BIG_QTY: i64 = 1_000_000_000;

/// Pin the current (Criterion timer) thread to a fixed core. Safe to
/// call once at the top of every bench fn; falls back to core 0 if the
/// box has fewer cores.
pub fn pin() {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let core = ids.get(BENCH_CORE).copied().unwrap_or(CoreId { id: 0 });
    core_affinity::set_for_current(core);
}

/// The one shared Criterion config. `sample_size(50)` matches the cast +
/// book benches so cross-crate numbers use the same statistics.
pub fn criterion() -> Criterion {
    Criterion::default().sample_size(50)
}

/// Symbol config shared by every fixture: tick 1, lot 1 => raw units,
/// so bench prices/qtys are the fixed-point values directly.
pub fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYMBOL_ID,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

/// One incoming order. `reduce_only` / `post_only` default false — set
/// them on the returned struct when a bench needs those paths.
pub fn order(
    price: i64,
    qty: i64,
    side: Side,
    tif: TimeInForce,
    user_id: u32,
    oid: u64,
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif,
        user_id,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1_700_000_000_000_000_000,
        order_id_hi: 0,
        order_id_lo: oid,
    }
}

/// A book holding `depth` resting asks laddered up from `MID + 1`, each
/// with `BIG_QTY` size. Best ask is always `MID + 1`, so a qty-1 taker
/// buy at `MID + 1` does exactly one non-draining partial fill no matter
/// how deep the book is — the match work is held constant while depth
/// varies. Deterministic: a given `depth` always yields the same book.
///
/// Prices beyond `MID + ~5%` fall outside the 1:1 compression zone and
/// share tick slots (the book's sawtooth index); those deep orders are
/// pure depth filler the taker never touches.
pub fn build_book(depth: u64) -> Orderbook {
    let cap = (depth + 1_024) as u32;
    let mut book = Orderbook::new(config(), cap, MID);
    for i in 0..depth {
        book.insert_resting(
            MID + 1 + i as i64,
            BIG_QTY,
            Side::Sell,
            0,
            200 + (i % 4096) as u32,
            false,
            1,
            0,
            2_000 + i,
        );
    }
    book
}

/// A minimal book with a single resting ask of `qty` at `MID + 1`, used
/// by the by-order-type benches that need exactly one level to cross.
pub fn single_ask(qty: i64) -> Orderbook {
    let mut book = Orderbook::new(config(), 1_024, MID);
    book.insert_resting(MID + 1, qty, Side::Sell, 0, 200, false, 1, 0, 2_000);
    book
}

/// The full ME critical section as one reusable fixture: real
/// `Orderbook` (seeded to a depth), real `WalWriter` (tempdir-backed),
/// real `DedupTracker`, real FxHashMap order index. `accept()` runs the
/// exact sequence the ME main loop runs between `me_in` and `me_out`
/// (sans cast send): dedup check + `OrderAcceptedRecord` WAL append +
/// `process_new_order` + `write_events_to_wal` + order-index update.
pub struct Me {
    pub book: Orderbook,
    pub wal: WalWriter,
    pub dedup: DedupTracker,
    pub index: FxHashMap<OrderKey, u32>,
    counter: u64,
    _tmp: TempDir,
}

impl Me {
    /// Seed an ME whose book holds `depth` resting asks (best `MID + 1`,
    /// `BIG_QTY` each) so `accept()` always does one non-draining fill.
    pub fn new(depth: u64) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let wal = WalWriter::new(SYMBOL_ID, tmp.path(), 64 * 1024 * 1024).expect("wal");
        Me {
            book: build_book(depth),
            wal,
            dedup: DedupTracker::new(),
            index: FxHashMap::default(),
            counter: 1,
            _tmp: tmp,
        }
    }

    /// One full accept: IOC buy of qty 1 at the best ask -> one fill.
    /// Returns nothing; measure the whole call.
    pub fn accept(&mut self) {
        self.counter += 1;
        let user_id = 1_000 + (self.counter % 1024) as u32;
        let oid = self.counter;

        let is_dup = self.dedup.check_and_insert(user_id, 0, oid);
        criterion::black_box(is_dup);

        let mut accepted = OrderAcceptedRecord {
            seq: 0,
            ts_ns: 1_700_000_000_000_000_000,
            user_id,
            symbol_id: SYMBOL_ID,
            order_id_hi: 0,
            order_id_lo: oid,
            price: MID + 1,
            qty: 1,
            side: 0,
            tif: 1, // IOC
            reduce_only: 0,
            post_only: 0,
            cid: [0; 20],
        };
        {
            let framed = self.wal.prepare(&mut accepted).expect("prepare");
            self.wal.append_framed(&framed).expect("append");
        }

        let mut incoming = order(MID + 1, 1, Side::Buy, TimeInForce::IOC, user_id, oid);
        process_new_order(&mut self.book, &mut incoming);

        write_events_to_wal(
            &mut self.wal,
            &self.book,
            SYMBOL_ID,
            1_700_000_000_000_000_000,
        )
        .expect("write events");

        // Drain the WAL buffer periodically WITHOUT fsync so it stays
        // bounded across the run; the per-order hot path is only the
        // memcpy into the buffer (fsync is batched off-path every 10ms).
        if self.counter.is_multiple_of(1024) {
            self.wal.reset_write_buf();
        }

        update_order_index(self.book.events(), &mut self.index);
    }
}
