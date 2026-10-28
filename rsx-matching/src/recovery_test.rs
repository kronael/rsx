use crate::checkpoint::save_checkpoint;
use crate::config::BootConfig;
use crate::recovery::recover;
use crate::recovery::Recovered;
use crate::recovery::RecoveryError;
use crate::recovery::BOOK_ORDER_CAPACITY;
use crate::wal::rebuild_dedup_window;
use rsx_book::book::Orderbook;
use rsx_cast::wal::WalWriter;
use rsx_cast::CastRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::TempDir;

const SYM: u32 = 1;
const NOW_NS: u64 = 1;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYM,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn wal_dir(dir: &TempDir) -> &str {
    dir.path().to_str().unwrap()
}

fn active_wal_file(dir: &TempDir) -> PathBuf {
    dir.path()
        .join(SYM.to_string())
        .join(format!("{SYM}_active.wal"))
}

/// A record whose payload is 16 bytes, shorter than every ME record type.
#[derive(Clone, Copy)]
struct Short<const RECORD_TYPE: u16> {
    seq: u64,
    _pad: u64,
}

impl<const RECORD_TYPE: u16> Short<RECORD_TYPE> {
    fn new() -> Self {
        Self { seq: 0, _pad: 0 }
    }
}

impl<const RECORD_TYPE: u16> CastRecord for Short<RECORD_TYPE> {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_TYPE
    }
}

fn append<T: CastRecord>(writer: &mut WalWriter, record: &mut T) {
    let framed = writer.prepare(record).unwrap();
    writer.append_framed(&framed).unwrap();
}

fn accepted(user_id: u32, order_id_lo: u64, price: i64) -> OrderAcceptedRecord {
    OrderAcceptedRecord {
        seq: 0,
        ts_ns: NOW_NS,
        user_id,
        symbol_id: SYM,
        order_id_hi: 0,
        order_id_lo,
        price,
        qty: 5,
        side: 0,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        cid: [0; 20],
        _pad: [0; 48],
    }
}

fn resting_orders(book: &Orderbook) -> usize {
    (0..book.orders.len())
        .filter(|&handle| book.orders.get(handle).is_active())
        .count()
}

fn recover_clean(dir: &TempDir) -> Recovered {
    recover(BootConfig::env(config()), wal_dir(dir), NOW_NS).unwrap()
}

#[test]
fn empty_wal_dir_boots_clean_at_seq_1() {
    let dir = TempDir::new().unwrap();
    let recovered = recover_clean(&dir);
    assert_eq!(recovered.next_seq, 1);
    assert_eq!(recovered.checkpoint_wal_seq, 0);
    assert_eq!(resting_orders(&recovered.book), 0);
    assert!(recovered.order_index.is_empty());
    assert_eq!(recovered.dedup.len(), 0);
}

#[test]
fn clean_wal_rebuilds_book_index_dedup_and_next_seq() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut accepted(10, 1, 100));
    append(&mut writer, &mut accepted(20, 2, 99));
    writer.flush().unwrap();
    drop(writer);

    let mut recovered = recover_clean(&dir);
    assert_eq!(resting_orders(&recovered.book), 2);
    assert_eq!(recovered.order_index.len(), 2);
    assert_eq!(recovered.dedup.len(), 2);
    assert!(recovered.dedup.check_and_insert(10, 0, 1));
    assert_eq!(recovered.next_seq, 3, "one past the last seq on disk");
}

#[test]
fn snapshot_without_newer_wal_continues_at_checkpoint_seq_plus_one() {
    let dir = TempDir::new().unwrap();
    let mut book = Orderbook::new(config(), 1024, 50_000);
    book.insert_resting(100, 5, rsx_types::Side::Buy, 0, 10, false, 1, 0, 1);
    save_checkpoint(&book, wal_dir(&dir), SYM, 17).unwrap();

    let recovered = recover_clean(&dir);
    assert_eq!(recovered.next_seq, 18);
    assert_eq!(recovered.checkpoint_wal_seq, 17);
    assert_eq!(resting_orders(&recovered.book), 1);
    assert_eq!(recovered.order_index.len(), 1);
}

#[test]
fn snapshot_plus_newer_wal_continues_after_the_last_wal_seq() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut accepted(10, 1, 100));
    writer.flush().unwrap();
    let mut book = Orderbook::new(config(), 1024, 50_000);
    book.insert_resting(100, 5, rsx_types::Side::Buy, 0, 10, false, 1, 0, 1);
    save_checkpoint(&book, wal_dir(&dir), SYM, writer.last_seq()).unwrap();
    append(&mut writer, &mut accepted(20, 2, 99));
    append(&mut writer, &mut accepted(20, 3, 98));
    writer.flush().unwrap();
    drop(writer);

    let recovered = recover_clean(&dir);
    assert_eq!(resting_orders(&recovered.book), 3);
    assert_eq!(recovered.next_seq, 4);
}

fn applied_config() -> BootConfig {
    BootConfig {
        config: SymbolConfig {
            symbol_id: SYM,
            price_decimals: 2,
            qty_decimals: 3,
            tick_size: 10,
            lot_size: 5,
        },
        applied_version: Some(2),
        book_capacity: BOOK_ORDER_CAPACITY,
    }
}

#[test]
fn a_checkpoint_keeps_its_config_over_an_older_applied_row() {
    let dir = TempDir::new().unwrap();
    let mut newer = config();
    newer.tick_size = 2;
    let mut book = Orderbook::new(newer, 1024, 50_000);
    book.insert_resting(100, 5, rsx_types::Side::Buy, 0, 10, false, 1, 0, 1);
    save_checkpoint(&book, wal_dir(&dir), SYM, 17).unwrap();

    let older = BootConfig {
        config: config(),
        applied_version: Some(2),
        book_capacity: BOOK_ORDER_CAPACITY,
    };
    let recovered = recover(older, wal_dir(&dir), NOW_NS).unwrap();
    assert_eq!(recovered.book.config.tick_size, 2);
    assert_eq!(resting_orders(&recovered.book), 1, "resting order kept");
}

#[test]
fn applied_config_builds_the_fresh_book() {
    let dir = TempDir::new().unwrap();
    let recovered = recover(applied_config(), wal_dir(&dir), NOW_NS).unwrap();
    assert_eq!(recovered.book.config.tick_size, 10);
    assert_eq!(recovered.book.config.lot_size, 5);
}

#[test]
fn env_config_leaves_the_checkpoint_config() {
    let dir = TempDir::new().unwrap();
    let mut other = config();
    other.tick_size = 4;
    save_checkpoint(&Orderbook::new(other, 1024, 50_000), wal_dir(&dir), SYM, 17).unwrap();

    let recovered = recover_clean(&dir);
    assert_eq!(recovered.book.config.tick_size, 4);
}

#[test]
fn unreadable_wal_stops_recovery_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut accepted(10, 1, 100));
    writer.flush().unwrap();
    drop(writer);
    // A header with an unknown version byte after a good record.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(active_wal_file(&dir))
        .unwrap();
    file.write_all(&[0xff; 64]).unwrap();

    let error = match recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS) {
        Ok(_) => panic!("recovery must not succeed past an unreadable WAL"),
        Err(error) => error,
    };
    let RecoveryError::ReplayRead { start_seq, .. } = error else {
        panic!("expected ReplayRead, got {error}");
    };
    assert_eq!(start_seq, 1);
}

fn write_three_accepted(dir: &TempDir) -> u64 {
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    for lo in 1..=3 {
        append(&mut writer, &mut accepted(10, lo, 100));
    }
    writer.flush().unwrap();
    std::fs::metadata(active_wal_file(dir)).unwrap().len() / 3
}

/// A record that fails its CRC with intact records after it must stop boot:
/// replay ending there would leave out the orders that follow and the live
/// writer would reuse their seqs.
#[test]
fn mid_file_crc_failure_stops_recovery() {
    let dir = TempDir::new().unwrap();
    let frame = write_three_accepted(&dir);
    let path = active_wal_file(&dir);
    let mut data = std::fs::read(&path).unwrap();
    data[frame as usize + 16] ^= 0xff;
    std::fs::write(&path, data).unwrap();

    let error = match recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS) {
        Ok(_) => panic!("recovery must not succeed past a corrupt record"),
        Err(error) => error,
    };
    let RecoveryError::ReplayRead { start_seq, .. } = error else {
        panic!("expected ReplayRead, got {error}");
    };
    assert_eq!(start_seq, 1);
}

/// A crash mid-write tears the last record of the active segment: recovery
/// keeps the records before it and continues at the seq after them.
#[test]
fn torn_tail_recovers_the_records_before_it() {
    let dir = TempDir::new().unwrap();
    let frame = write_three_accepted(&dir);
    std::fs::OpenOptions::new()
        .write(true)
        .open(active_wal_file(&dir))
        .unwrap()
        .set_len(2 * frame + 10)
        .unwrap();

    let recovered = recover_clean(&dir);
    assert_eq!(resting_orders(&recovered.book), 2);
    assert_eq!(recovered.next_seq, 3);
}

#[test]
fn truncated_accepted_payload_stops_recovery() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut accepted(10, 1, 100));
    append(&mut writer, &mut Short::<RECORD_ORDER_ACCEPTED>::new());
    writer.flush().unwrap();
    drop(writer);

    let error = match recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS) {
        Ok(_) => panic!("recovery must not skip an undecodable ORDER_ACCEPTED"),
        Err(error) => error,
    };
    let RecoveryError::UndecodableRecord {
        record_type,
        seq,
        payload_len,
        expected_len,
    } = error
    else {
        panic!("expected UndecodableRecord, got {error}");
    };
    assert_eq!(record_type, RECORD_ORDER_ACCEPTED);
    assert_eq!(seq, Some(2));
    assert_eq!(payload_len, 16);
    assert_eq!(expected_len, std::mem::size_of::<OrderAcceptedRecord>());
}

#[test]
fn truncated_cancelled_payload_stops_recovery() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut accepted(10, 1, 100));
    append(&mut writer, &mut Short::<RECORD_ORDER_CANCELLED>::new());
    writer.flush().unwrap();
    drop(writer);

    let error = match recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS) {
        Ok(_) => panic!("recovery must not skip an undecodable ORDER_CANCELLED"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            RecoveryError::UndecodableRecord {
                record_type: RECORD_ORDER_CANCELLED,
                ..
            }
        ),
        "got {error}",
    );
}

#[test]
fn truncated_accepted_before_the_checkpoint_still_stops_dedup_rebuild() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut Short::<RECORD_ORDER_ACCEPTED>::new());
    writer.flush().unwrap();
    let book = Orderbook::new(config(), 1024, 50_000);
    save_checkpoint(&book, wal_dir(&dir), SYM, writer.last_seq()).unwrap();
    drop(writer);

    let mut dedup = crate::dedup::DedupTracker::new();
    let error = rebuild_dedup_window(&mut dedup, wal_dir(&dir), SYM, NOW_NS).unwrap_err();
    assert!(matches!(
        error,
        RecoveryError::UndecodableRecord {
            record_type: RECORD_ORDER_ACCEPTED,
            ..
        }
    ));
    assert!(recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS).is_err());
}

#[test]
fn error_message_names_the_record() {
    let error = RecoveryError::UndecodableRecord {
        record_type: RECORD_ORDER_ACCEPTED,
        seq: Some(41),
        payload_len: 16,
        expected_len: 128,
    };
    assert_eq!(
        error.to_string(),
        "wal record type 7 seq 41 has a 16-byte payload, expected 128",
    );
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn recover_logging(wal_dir: &Path) -> (Recovered, String) {
    let buffer = LogBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let recovered = tracing::subscriber::with_default(subscriber, || {
        recover(BootConfig::env(config()), wal_dir.to_str().unwrap(), NOW_NS).unwrap()
    });
    let log = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    (recovered, log)
}

#[test]
fn unknown_record_types_are_skipped_with_one_warning_per_type() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut Short::<99>::new());
    append(&mut writer, &mut accepted(10, 1, 100));
    append(&mut writer, &mut Short::<99>::new());
    append(&mut writer, &mut Short::<98>::new());
    writer.flush().unwrap();
    drop(writer);

    let (recovered, log) = recover_logging(dir.path());
    assert_eq!(resting_orders(&recovered.book), 1);
    assert_eq!(recovered.next_seq, 5);
    assert_eq!(log.matches("unknown record type 99").count(), 1, "{log}");
    assert_eq!(log.matches("unknown record type 98").count(), 1, "{log}");
}

#[test]
fn known_side_effect_records_are_replayed_without_a_warning() {
    let dir = TempDir::new().unwrap();
    let mut writer = WalWriter::new(SYM, dir.path(), 64 * 1024 * 1024).unwrap();
    append(&mut writer, &mut accepted(10, 1, 100));
    append(
        &mut writer,
        &mut rsx_messages::OrderDoneRecord {
            seq: 0,
            ts_ns: 1,
            symbol_id: SYM,
            user_id: 10,
            order_id_hi: 0,
            order_id_lo: 1,
            filled_qty: rsx_types::Qty(0),
            remaining_qty: rsx_types::Qty(5),
            final_status: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
            gw_in_ns: 0,
            risk_in_ns: 0,
            me_in_ns: 0,
            match_done_ns: 0,
            gw_out_ns: 0,
            _pad2: [0; 24],
        },
    );
    writer.flush().unwrap();
    drop(writer);

    let (recovered, log) = recover_logging(dir.path());
    assert_eq!(recovered.next_seq, 3);
    assert!(!log.contains("unknown record type"), "{log}");
}

/// Rests one buy order per slot across 100 price levels and 1000 users.
fn rest_one(book: &mut Orderbook, n: u32) {
    book.insert_resting(
        49_900 + i64::from(n % 100),
        1,
        Side::Buy,
        0,
        n % 1000 + 1,
        false,
        1,
        0,
        u64::from(n) + 1,
    );
}

/// Fills every slot of a fresh book with a resting order.
fn assert_fills_to_the_limit(book: &mut Orderbook, capacity: u32) {
    assert_eq!(book.orders.capacity(), capacity);
    for n in 0..capacity {
        rest_one(book, n);
    }
    assert_eq!(resting_orders(book), capacity as usize);
}

#[test]
fn fresh_book_holds_a_million_resting_orders() {
    assert_eq!(BOOK_ORDER_CAPACITY, 1_000_000);
    let dir = TempDir::new().unwrap();
    let mut recovered = recover_clean(&dir);
    assert_fills_to_the_limit(&mut recovered.book, 1_000_000);
}

/// The order past the last slot trips the slab's invariant.
#[test]
#[should_panic(expected = "slab exhausted")]
fn the_order_past_a_million_resting_orders_panics() {
    let dir = TempDir::new().unwrap();
    let mut recovered = recover_clean(&dir);
    assert_fills_to_the_limit(&mut recovered.book, 1_000_000);
    rest_one(&mut recovered.book, 1_000_000);
}

#[test]
fn configured_capacity_sizes_the_fresh_book() {
    let dir = TempDir::new().unwrap();
    let boot = BootConfig {
        book_capacity: 64,
        ..BootConfig::env(config())
    };
    let mut recovered = recover(boot, wal_dir(&dir), NOW_NS).unwrap();
    assert_fills_to_the_limit(&mut recovered.book, 64);
}

#[test]
#[should_panic(expected = "slab exhausted")]
fn the_order_past_a_configured_capacity_panics() {
    let dir = TempDir::new().unwrap();
    let boot = BootConfig {
        book_capacity: 64,
        ..BootConfig::env(config())
    };
    let mut recovered = recover(boot, wal_dir(&dir), NOW_NS).unwrap();
    assert_fills_to_the_limit(&mut recovered.book, 64);
    rest_one(&mut recovered.book, 64);
}

#[test]
fn a_checkpoint_restores_at_the_boot_capacity() {
    let dir = TempDir::new().unwrap();
    save_checkpoint(
        &Orderbook::new(config(), 128, 50_000),
        wal_dir(&dir),
        SYM,
        17,
    )
    .unwrap();
    let recovered = recover_clean(&dir);
    assert_eq!(recovered.book.orders.capacity(), BOOK_ORDER_CAPACITY);
}

#[test]
fn a_checkpoint_never_shrinks_below_the_capacity_it_was_written_with() {
    let dir = TempDir::new().unwrap();
    save_checkpoint(
        &Orderbook::new(config(), 128, 50_000),
        wal_dir(&dir),
        SYM,
        17,
    )
    .unwrap();
    let boot = BootConfig {
        book_capacity: 64,
        ..BootConfig::env(config())
    };
    let recovered = recover(boot, wal_dir(&dir), NOW_NS).unwrap();
    assert_eq!(recovered.book.orders.capacity(), 128);
}

#[test]
fn an_unreadable_checkpoint_stops_recovery() {
    let dir = TempDir::new().unwrap();
    let symbol_dir = dir.path().join(SYM.to_string());
    std::fs::create_dir_all(&symbol_dir).unwrap();
    std::fs::write(symbol_dir.join("snapshot.bin"), b"not a checkpoint").unwrap();
    let Err(error) = recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS) else {
        panic!("a garbage checkpoint must not boot an empty book");
    };
    assert!(matches!(error, RecoveryError::CheckpointLoad { .. }));
}

#[test]
fn a_short_checkpoint_stops_recovery() {
    let dir = TempDir::new().unwrap();
    let symbol_dir = dir.path().join(SYM.to_string());
    std::fs::create_dir_all(&symbol_dir).unwrap();
    std::fs::write(symbol_dir.join("snapshot.bin"), b"ab").unwrap();
    let Err(error) = recover(BootConfig::env(config()), wal_dir(&dir), NOW_NS) else {
        panic!("a short checkpoint must not boot an empty book");
    };
    assert!(matches!(error, RecoveryError::CheckpointLoad { .. }));
}
