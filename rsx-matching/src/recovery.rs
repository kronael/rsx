//! Boot recovery: checkpoint restore, WAL replay and dedup-window rebuild.
//! Any failure is a named [`RecoveryError`]; the caller stops boot on it
//! (`specs/2/6-consistency.md` § 5: restart is the one recovery path), so a
//! half-rebuilt book never trades.

use crate::checkpoint::load_checkpoint;
use crate::dedup::DedupTracker;
use crate::wal::rebuild_dedup_window;
use crate::wal::rebuild_order_index_from_book;
use crate::wal::replay_wal_after_snapshot;
use crate::wal::OrderKey;
use rsx_book::book::Orderbook;
use rsx_types::SymbolConfig;
use rustc_hash::FxHashMap;
use std::fmt;
use std::io;
use tracing::info;

/// Order slab capacity of a book built without a checkpoint.
pub const BOOK_ORDER_CAPACITY: u32 = 65_536;
/// Initial mid price of a book built without a checkpoint.
pub const BOOK_INITIAL_MID_PRICE: i64 = 50_000;

/// Why recovery stopped.
#[derive(Debug)]
pub enum RecoveryError {
    /// The WAL could not be read while replaying from `start_seq`.
    ReplayRead { start_seq: u64, source: io::Error },
    /// The WAL could not be read while rebuilding the dedup window.
    DedupRead { source: io::Error },
    /// A record of a type recovery applies has a payload shorter than that
    /// record: corruption or a framing bug.
    UndecodableRecord {
        record_type: u16,
        seq: Option<u64>,
        payload_len: usize,
        expected_len: usize,
    },
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReplayRead { start_seq, source } => {
                write!(f, "wal replay from seq {start_seq} failed: {source}")
            }
            Self::DedupRead { source } => {
                write!(f, "dedup window rebuild failed: {source}")
            }
            Self::UndecodableRecord {
                record_type,
                seq,
                payload_len,
                expected_len,
            } => match seq {
                Some(seq) => write!(
                    f,
                    "wal record type {record_type} seq {seq} has a \
                     {payload_len}-byte payload, expected {expected_len}"
                ),
                None => write!(
                    f,
                    "wal record type {record_type} has a {payload_len}-byte \
                     payload, expected {expected_len}"
                ),
            },
        }
    }
}

impl std::error::Error for RecoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReplayRead { source, .. } | Self::DedupRead { source } => Some(source),
            Self::UndecodableRecord { .. } => None,
        }
    }
}

/// State rebuilt from disk, ready for the live loop.
pub struct Recovered {
    pub book: Orderbook,
    pub order_index: FxHashMap<OrderKey, u32>,
    pub dedup: DedupTracker,
    /// First WAL seq the live writer assigns: one past everything on disk
    /// and everything the checkpoint covers, so seqs never repeat.
    pub next_seq: u64,
}

/// Rebuild the symbol's state from `wal_dir`:
///   1. Load the checkpoint with its embedded WAL replay boundary, or start
///      from an empty book built from `config` and replay from seq 1.
///   2. Replay WAL records after that boundary by re-executing
///      `OrderAccepted`, which regenerates fills deterministically.
///   3. Rebuild the dedup window from every retained `OrderAccepted`
///      (the checkpoint does not persist it).
///
/// `now_unix_ns` ages the dedup entries. An empty or missing WAL directory
/// is a clean boot, not an error.
pub fn recover(
    config: SymbolConfig,
    wal_dir: &str,
    now_unix_ns: u64,
) -> Result<Recovered, RecoveryError> {
    let symbol_id = config.symbol_id;
    let mut order_index: FxHashMap<OrderKey, u32> = FxHashMap::default();
    let (mut book, start_seq) = match load_checkpoint(wal_dir, symbol_id) {
        Some((loaded, wal_seq)) => {
            let book = *loaded;
            rebuild_order_index_from_book(&book, &mut order_index);
            info!(
                "book restored from checkpoint: book.seq={} wal_seq={}",
                book.sequence, wal_seq,
            );
            (book, wal_seq + 1)
        }
        None => {
            info!("no snapshot found — replaying WAL from seq 1");
            (
                Orderbook::new(config, BOOK_ORDER_CAPACITY, BOOK_INITIAL_MID_PRICE),
                1,
            )
        }
    };
    let last_seq =
        replay_wal_after_snapshot(&mut book, &mut order_index, wal_dir, symbol_id, start_seq)?;

    let mut dedup = DedupTracker::new();
    let seeded = rebuild_dedup_window(&mut dedup, wal_dir, symbol_id, now_unix_ns)?;
    info!("dedup window rebuilt from wal: {seeded} keys");

    Ok(Recovered {
        book,
        order_index,
        dedup,
        // `last_seq` is `start_seq - 1` when nothing newer is on disk.
        next_seq: last_seq + 1,
    })
}

#[cfg(test)]
#[path = "recovery_test.rs"]
mod recovery_test;
