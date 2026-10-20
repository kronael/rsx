use apache_avro::from_value;
use apache_avro::Reader;
use apache_avro::Schema;
use apache_avro::Writer;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderDoneReason;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::snapshot;
use rsx_book::snapshot::BookSnapshotV1;
use rsx_book::snapshot::SnapshotU64;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::Framed;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_types::Side;
use rsx_types::TimeInForce;
use rsx_types::NONE;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::Read as _;
use std::io::Seek as _;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

const LEGACY_CHECKPOINT_MAGIC: u32 = 0x5253_5843;
const LEGACY_CHECKPOINT_VERSION: u32 = 1;
const CHECKPOINT_SCHEMA: &str = include_str!("../checkpoint-v1.avsc");
const CHECKPOINT_SCHEMA_VERSION: i32 = 1;
const AVRO_MAGIC: [u8; 4] = *b"Obj\x01";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchingCheckpointV1 {
    pub schema_version: i32,
    pub wal_seq: SnapshotU64,
    pub book: BookSnapshotV1,
}

impl MatchingCheckpointV1 {
    pub fn new(wal_seq: u64, book: BookSnapshotV1) -> Self {
        Self {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            wal_seq: SnapshotU64::new(wal_seq),
            book,
        }
    }
}

enum CheckpointCommand {
    Persist(MatchingCheckpointV1),
    Shutdown,
}

pub struct CheckpointWriter {
    sender: mpsc::SyncSender<CheckpointCommand>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl CheckpointWriter {
    pub fn start(wal_dir: &str, symbol_id: u32) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let wal_dir = wal_dir.to_owned();
        let worker = thread::Builder::new()
            .name(format!("rsx-me-checkpoint-{symbol_id}"))
            .spawn(move || run_checkpoint_worker(receiver, wal_dir, symbol_id))?;
        Ok(Self {
            sender,
            worker: Some(worker),
        })
    }

    pub fn submit(&self, checkpoint: MatchingCheckpointV1) -> io::Result<()> {
        match self.sender.try_send(CheckpointCommand::Persist(checkpoint)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "checkpoint writer is still busy",
            )),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "checkpoint writer stopped",
            )),
        }
    }

    pub fn shutdown(mut self, final_checkpoint: Option<MatchingCheckpointV1>) -> io::Result<()> {
        let mut send_error = None;
        if let Some(checkpoint) = final_checkpoint {
            if self
                .sender
                .send(CheckpointCommand::Persist(checkpoint))
                .is_err()
            {
                send_error = Some(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "checkpoint writer stopped before final checkpoint",
                ));
            }
        }
        if self.sender.send(CheckpointCommand::Shutdown).is_err() && send_error.is_none() {
            send_error = Some(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "checkpoint writer stopped before shutdown",
            ));
        }
        drop(self.sender);

        let worker = self
            .worker
            .take()
            .ok_or_else(|| io::Error::other("checkpoint worker missing"))?;
        let result = worker
            .join()
            .map_err(|_| io::Error::other("checkpoint worker panicked"))?;
        result?;
        if let Some(error) = send_error {
            return Err(error);
        }
        Ok(())
    }
}

fn run_checkpoint_worker(
    receiver: mpsc::Receiver<CheckpointCommand>,
    wal_dir: String,
    symbol_id: u32,
) -> io::Result<()> {
    while let Ok(command) = receiver.recv() {
        match command {
            CheckpointCommand::Persist(checkpoint) => {
                persist_checkpoint(&checkpoint, &wal_dir, symbol_id)?;
            }
            CheckpointCommand::Shutdown => return Ok(()),
        }
    }
    Ok(())
}

/// `(user_id, order_id_hi, order_id_lo) -> slab handle`.
pub type OrderKey = (u32, u64, u64);

/// Translate the typed matching result into the web protocol's U-frame
/// status: 0 is FILLED and 2 is CANCELLED.
fn done_final_status(reason: OrderDoneReason) -> u8 {
    match reason {
        OrderDoneReason::Cancelled => 2,
        OrderDoneReason::Filled => 0,
    }
}

/// Write every event from the book's buffer to the WAL only (no cast).
/// Replay/bench helper — writes the SAME records as the production
/// fan-out (`publish_events`), including BBO; it just doesn't cast to
/// risk/marketdata.
pub fn write_events_to_wal(
    writer: &mut WalWriter,
    book: &Orderbook,
    symbol_id: u32,
    ts_ns: u64,
) -> io::Result<()> {
    emit_events(&mut WalSink { writer }, book, symbol_id, ts_ns, 0, 0, 0, 0)
}

/// Walk the book's event buffer once, build each event's wire record, and
/// hand it to `sink`. The record-construction match lives here and nowhere
/// else; the sink decides where the bytes go (WAL-only vs full fan-out).
///
/// `me_in_ns`/`match_done_ns` bound this match cycle (specs/2/
/// 59-latency-observability.md "engine" leg) and are stamped onto every
/// `FillRecord` this cycle emits — one match cycle, one pair of hop
/// timestamps, same as `ts_ns` already is. `0` means "not measured" (e.g.
/// replay/bench callers that pass 0).
#[allow(clippy::too_many_arguments)]
fn emit_events<S: EventSink>(
    sink: &mut S,
    book: &Orderbook,
    symbol_id: u32,
    ts_ns: u64,
    gw_in_ns: u64,
    risk_in_ns: u64,
    me_in_ns: u64,
    match_done_ns: u64,
) -> io::Result<()> {
    for event in book.events() {
        match *event {
            Event::Fill {
                maker_handle: _,
                maker_tif,
                maker_reduce_only,
                maker_user_id,
                taker_user_id,
                price,
                qty,
                side,
                maker_order_id_hi,
                maker_order_id_lo,
                taker_order_id_hi,
                taker_order_id_lo,
                gw_in_ns: event_gw_in_ns,
            } => {
                let mut record = FillRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    taker_user_id,
                    maker_user_id,
                    _pad0: 0,
                    taker_order_id_hi,
                    taker_order_id_lo,
                    maker_order_id_hi,
                    maker_order_id_lo,
                    price,
                    qty,
                    taker_side: side,
                    reduce_only: maker_reduce_only as u8,
                    tif: maker_tif,
                    post_only: 0,
                    _pad1: [0; 4],
                    gw_in_ns: event_gw_in_ns,
                    risk_in_ns,
                    me_in_ns,
                    match_done_ns,
                    // Stamped by the gateway just before it builds the
                    // outbound client frame, not here.
                    gw_out_ns: 0,
                };
                sink.emit(&mut record)?;
            }
            Event::OrderInserted {
                handle,
                user_id,
                side,
                price,
                qty,
                order_id_hi,
                order_id_lo,
            } => {
                let (reduce_only, tif) = if handle != NONE {
                    let slot = book.orders.get(handle);
                    (slot.is_reduce_only() as u8, slot.tif)
                } else {
                    (0, 0)
                };
                let mut record = OrderInsertedRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    price,
                    qty,
                    side,
                    reduce_only,
                    tif,
                    post_only: 0,
                    _pad1: [0; 4],
                };
                sink.emit(&mut record)?;
            }
            Event::OrderCancelled {
                handle,
                user_id,
                remaining_qty,
                reason,
                order_id_hi,
                order_id_lo,
            } => {
                let (reduce_only, tif, post_only) = if handle != NONE {
                    let slot = book.orders.get(handle);
                    (slot.is_reduce_only() as u8, slot.tif, 0u8)
                } else {
                    (0, 0, 0)
                };
                let mut record = OrderCancelledRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    remaining_qty,
                    reason: reason as u8,
                    reduce_only,
                    tif,
                    post_only,
                    _pad1: [0; 4],
                };
                sink.emit(&mut record)?;
            }
            Event::OrderDone {
                handle,
                tif,
                reduce_only,
                user_id,
                reason,
                filled_qty,
                remaining_qty,
                order_id_hi,
                order_id_lo,
            } => {
                let (gw_in_ns, risk_in_ns, me_in_ns, match_done_ns) = if handle == NONE {
                    (gw_in_ns, risk_in_ns, me_in_ns, match_done_ns)
                } else {
                    (0, 0, 0, 0)
                };
                let mut record = OrderDoneRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    filled_qty,
                    remaining_qty,
                    final_status: done_final_status(reason),
                    reduce_only: reduce_only as u8,
                    tif,
                    post_only: 0,
                    _pad1: [0; 4],
                    gw_in_ns,
                    risk_in_ns,
                    me_in_ns,
                    match_done_ns,
                    gw_out_ns: 0,
                };
                // SEQ-1: full fan-out to BOTH streams. Sending to
                // only one consumer leaves a WAL-seq hole on the
                // other → false FAULTED. marketdata ignores types
                // it doesn't handle.
                sink.emit(&mut record)?;
            }
            Event::OrderFailed {
                user_id,
                reason,
                order_id_hi,
                order_id_lo,
            } => {
                let mut record = OrderFailedRecord {
                    seq: 0,
                    ts_ns,
                    user_id,
                    _pad0: 0,
                    order_id_hi,
                    order_id_lo,
                    reason: reason as u8,
                    _pad: [0; 23],
                };
                // Every record consuming a WAL sequence is emitted to
                // both live streams so each stream remains contiguous.
                sink.emit(&mut record)?;
            }
            Event::BBO {
                bid_px,
                bid_qty,
                ask_px,
                ask_qty,
            } => {
                // BBO shares the WAL sequence space with every other
                // record. Replay re-derives BBO, but reserving its
                // sequence keeps both live streams contiguous.
                let mut record = rsx_messages::BboRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    _pad0: 0,
                    bid_px,
                    bid_qty,
                    bid_count: 0,
                    _pad1: 0,
                    ask_px,
                    ask_qty,
                    ask_count: 0,
                    _pad2: 0,
                };
                sink.emit(&mut record)?;
            }
        }
    }
    Ok(())
}

/// Publish each event once (WAL prepare = single CRC + seq), then fan out to
/// WAL + risk + marketdata. Production path. See ARCHITECTURE.md.
///
/// `me_in_ns`/`match_done_ns`: this match cycle's engine-leg hop
/// timestamps (specs/2/59-latency-observability.md), stamped onto every
/// `FillRecord` emitted this cycle. Pass `0` if not measured (e.g. the
/// cancel path, which has no `me_in`/match cycle of its own).
#[allow(clippy::too_many_arguments)]
pub fn publish_events(
    writer: &mut WalWriter,
    cmp: &mut CastSender,
    mkt: &mut CastSender,
    book: &Orderbook,
    symbol_id: u32,
    ts_ns: u64,
    gw_in_ns: u64,
    risk_in_ns: u64,
    me_in_ns: u64,
    match_done_ns: u64,
) -> io::Result<()> {
    emit_events(
        &mut FanoutSink { writer, cmp, mkt },
        book,
        symbol_id,
        ts_ns,
        gw_in_ns,
        risk_in_ns,
        me_in_ns,
        match_done_ns,
    )
}

/// Sink for the wire records built from a match cycle's event buffer.
/// `emit_events` builds each record once and hands it to the sink, so the
/// record-construction match lives in exactly one place.
trait EventSink {
    fn emit<T: rsx_cast::CastRecord>(&mut self, record: &mut T) -> io::Result<()>;
}

/// WAL-only sink: prepare (assign seq + CRC), then append. Replay/bench helper.
struct WalSink<'a> {
    writer: &'a mut WalWriter,
}

impl EventSink for WalSink<'_> {
    fn emit<T: rsx_cast::CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        let framed = self.writer.prepare(record)?;
        self.writer.append_framed(&framed)
    }
}

/// Production sink: frame once (single CRC + seq), then fan out to WAL + risk
/// + marketdata. See ARCHITECTURE.md.
struct FanoutSink<'a> {
    writer: &'a mut WalWriter,
    cmp: &'a mut CastSender,
    mkt: &'a mut CastSender,
}

impl EventSink for FanoutSink<'_> {
    fn emit<T: rsx_cast::CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        let framed: Framed = self.writer.prepare(record)?;
        self.writer.append_framed(&framed)?;
        self.cmp.send_framed(&framed)?;
        self.mkt.send_framed(&framed)
    }
}

/// Flush WAL if 10ms have elapsed since last flush. `now` is the caller's
/// cached loop clock, so the flush check costs no `Instant::now()`.
pub fn flush_if_due(
    writer: &mut WalWriter,
    last_flush: &mut Instant,
    now: Instant,
) -> io::Result<()> {
    if now.duration_since(*last_flush).as_millis() >= 10 {
        writer.flush()?;
        *last_flush = now;
    }
    Ok(())
}

fn checkpoint_path(wal_dir: &str, symbol_id: u32) -> PathBuf {
    PathBuf::from(wal_dir)
        .join(symbol_id.to_string())
        .join("snapshot.bin")
}

fn avro_error(error: apache_avro::Error) -> io::Error {
    io::Error::other(error)
}

fn read_avro_checkpoint(file: fs::File) -> io::Result<MatchingCheckpointV1> {
    let mut reader = Reader::new(file).map_err(avro_error)?;
    let value = reader
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty checkpoint"))?
        .map_err(avro_error)?;
    let checkpoint: MatchingCheckpointV1 = from_value(&value).map_err(avro_error)?;
    if checkpoint.schema_version != CHECKPOINT_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported checkpoint schema version",
        ));
    }
    if let Some(value) = reader.next() {
        value.map_err(avro_error)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "checkpoint contains multiple records",
        ));
    }
    Ok(checkpoint)
}

fn persist_checkpoint(
    checkpoint: &MatchingCheckpointV1,
    wal_dir: &str,
    symbol_id: u32,
) -> io::Result<()> {
    let dir = PathBuf::from(wal_dir).join(symbol_id.to_string());
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("snapshot.bin.tmp");
    let dest = dir.join("snapshot.bin");
    let schema = Schema::parse_str(CHECKPOINT_SCHEMA).map_err(avro_error)?;
    let file = fs::File::create(&tmp)?;
    let mut writer = Writer::new(&schema, file);
    writer.append_ser(checkpoint).map_err(avro_error)?;
    let file = writer.into_inner().map_err(avro_error)?;
    file.sync_all()?;
    fs::rename(&tmp, &dest)?;
    fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

/// Load an Avro checkpoint or a legacy v1 envelope with RXSN v2/v3 state.
pub fn load_checkpoint(wal_dir: &str, symbol_id: u32) -> Option<(Box<Orderbook>, u64)> {
    let path = checkpoint_path(wal_dir, symbol_id);
    let mut file = fs::File::open(&path).ok()?;
    let mut magic = [0u8; 4];
    if let Err(error) = file.read_exact(&mut magic) {
        warn!("checkpoint header load failed: {error}");
        return None;
    }
    if let Err(error) = file.rewind() {
        warn!("checkpoint rewind failed: {error}");
        return None;
    }

    let loaded = if magic == AVRO_MAGIC {
        read_avro_checkpoint(file).and_then(|checkpoint| {
            let wal_seq = checkpoint.wal_seq.get();
            let book = snapshot::restore(checkpoint.book)?;
            Ok((book, wal_seq))
        })
    } else {
        let mut header = [0u8; 16];
        file.read_exact(&mut header).and_then(|()| {
            let header_magic = u32::from_le_bytes(
                header[0..4]
                    .try_into()
                    .map_err(|_| io::Error::other("invalid checkpoint magic"))?,
            );
            let version = u32::from_le_bytes(
                header[4..8]
                    .try_into()
                    .map_err(|_| io::Error::other("invalid checkpoint version"))?,
            );
            if header_magic != LEGACY_CHECKPOINT_MAGIC || version != LEGACY_CHECKPOINT_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported legacy checkpoint header",
                ));
            }
            let wal_seq = u64::from_le_bytes(
                header[8..16]
                    .try_into()
                    .map_err(|_| io::Error::other("invalid checkpoint sequence"))?,
            );
            snapshot::load(&mut file).map(|book| (book, wal_seq))
        })
    };

    match loaded {
        Ok((book, wal_seq)) => {
            info!(wal_seq, "loaded checkpoint from {}", path.display());
            Some((book, wal_seq))
        }
        Err(error) => {
            warn!("checkpoint load failed: {error}, starting empty");
            None
        }
    }
}

/// Capture and synchronously persist one Avro checkpoint.
pub fn save_checkpoint(
    book: &Orderbook,
    wal_dir: &str,
    symbol_id: u32,
    wal_last_seq: u64,
) -> io::Result<()> {
    let checkpoint = MatchingCheckpointV1::new(wal_last_seq, snapshot::capture(book)?);
    persist_checkpoint(&checkpoint, wal_dir, symbol_id)
}

/// Replay WAL records after a snapshot to bring the book to current state.
/// Returns the highest WAL seq applied. See ARCHITECTURE.md.
pub fn replay_wal_after_snapshot(
    book: &mut Orderbook,
    order_index: &mut FxHashMap<OrderKey, u32>,
    wal_dir: &str,
    symbol_id: u32,
    start_seq: u64,
) -> io::Result<u64> {
    let wal_path = PathBuf::from(wal_dir);
    let mut reader = WalReader::open_from_seq(symbol_id, start_seq, &wal_path)?;
    let mut last_seq = start_seq.saturating_sub(1);
    let mut accepted = 0u64;
    let mut cancelled = 0u64;
    while let Some(raw) = reader.next()? {
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        if seq < start_seq {
            // `open_from_seq` picks the file containing
            // `start_seq` but doesn't skip past records
            // within that file. Filter here so we don't
            // re-apply records the snapshot already contains.
            continue;
        }
        if seq > last_seq {
            last_seq = seq;
        }
        match raw.header.record_type {
            t if t == RECORD_ORDER_ACCEPTED => {
                let Some(rec) = decode_payload::<OrderAcceptedRecord>(&raw.payload) else {
                    continue;
                };
                // Dedup is NOT re-seeded here — `rebuild_dedup_window`
                // owns the whole 1 h window (pre- AND post-snapshot) in
                // one ascending-seq pass, which keeps the pruning queue
                // ordered. This forward pass only reconstructs book + index.
                let mut incoming = IncomingOrder {
                    price: rec.price,
                    qty: rec.qty,
                    remaining_qty: rec.qty,
                    side: if rec.side == 0 { Side::Buy } else { Side::Sell },
                    tif: match rec.tif {
                        1 => TimeInForce::IOC,
                        2 => TimeInForce::FOK,
                        _ => TimeInForce::GTC,
                    },
                    user_id: rec.user_id,
                    reduce_only: rec.reduce_only != 0,
                    post_only: rec.post_only != 0,
                    timestamp_ns: rec.ts_ns,
                    order_id_hi: rec.order_id_hi,
                    order_id_lo: rec.order_id_lo,
                };
                process_new_order(book, &mut incoming);
                update_order_index(book.events(), order_index);
                accepted += 1;
            }
            t if t == RECORD_ORDER_CANCELLED => {
                let Some(rec) = decode_payload::<OrderCancelledRecord>(&raw.payload) else {
                    continue;
                };
                let key: OrderKey = (rec.user_id, rec.order_id_hi, rec.order_id_lo);
                if let Some(&handle) = order_index.get(&key) {
                    book.cancel_order(handle);
                    order_index.remove(&key);
                    cancelled += 1;
                }
            }
            _ => {} // Skip Fill / OrderInserted / OrderDone /
                    // OrderFailed / BBO — all side effects.
        }
    }
    info!(
        "wal replay: accepted={} cancelled={} last_seq={}",
        accepted, cancelled, last_seq,
    );
    Ok(last_seq)
}

/// Rebuild the full dedup window from the WAL on recovery.
///
/// A book snapshot persists the slab but NOT the dedup set, and the
/// snapshot cadence (~10 s) is far shorter than the dedup window
/// (`DEDUP_WINDOW`, 1 h). Restoring dedup only from post-snapshot
/// `RECORD_ORDER_ACCEPTED` records (as the book replay does) would leave
/// every order accepted more than one snapshot interval before the crash
/// unprotected — a legitimate client resend of its `(user_id, order_id)`
/// within the window would then be treated as new and double-execute,
/// violating exactly-one-completion.
///
/// Scan the retained WAL for `RECORD_ORDER_ACCEPTED` and seed each key
/// still inside the window with its remaining TTL (`seed` skips the rest,
/// keyed off the record's ME-stamped `ts_ns` vs `now_unix_ns`). Scanning
/// in seq order feeds `seed` oldest-first, keeping the pruning queue
/// ordered. Cold path, one-time; bounded by WAL retention (4 h ≫ 300 s).
/// Covers records on both sides of the checkpoint and is idempotent.
pub fn rebuild_dedup_window(
    dedup: &mut crate::dedup::DedupTracker,
    wal_dir: &str,
    symbol_id: u32,
    now_unix_ns: u64,
) -> io::Result<u64> {
    let wal_path = PathBuf::from(wal_dir);
    // Filter by ts_ns, not seq, so start from the earliest retained file.
    let mut reader = WalReader::open_from_seq(symbol_id, 0, &wal_path)?;
    let before = dedup.len();
    while let Some(raw) = reader.next()? {
        if raw.header.record_type != RECORD_ORDER_ACCEPTED {
            continue;
        }
        let Some(rec) = decode_payload::<OrderAcceptedRecord>(&raw.payload) else {
            continue;
        };
        let age_ns = now_unix_ns.saturating_sub(rec.ts_ns);
        dedup.seed(
            rec.user_id,
            rec.order_id_hi,
            rec.order_id_lo,
            Duration::from_nanos(age_ns),
        );
    }
    let seeded = dedup.len().saturating_sub(before) as u64;
    info!("dedup window rebuild: seeded={} keys from wal", seeded);
    Ok(seeded)
}

/// The one order-index maintainer, shared by the live loop
/// (`main.rs`), replay (below), and the bench harness: a
/// successful insert/restore puts the handle into the index; a
/// Done removes it.
pub fn update_order_index(events: &[Event], index: &mut FxHashMap<OrderKey, u32>) {
    for ev in events {
        match *ev {
            Event::OrderInserted {
                handle,
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.insert((user_id, order_id_hi, order_id_lo), handle);
            }
            Event::OrderDone {
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.remove(&(user_id, order_id_hi, order_id_lo));
            }
            _ => {}
        }
    }
}

/// Rebuild `order_index` from the active resting orders restored from a
/// logical snapshot. Recovery derives the `(user, order ID) -> handle`
/// map before accepting cancels.
pub fn rebuild_order_index_from_book(book: &Orderbook, index: &mut FxHashMap<OrderKey, u32>) {
    index.clear();
    for handle in 0..book.orders.len() {
        let slot = book.orders.get(handle);
        if !slot.is_active() {
            continue;
        }
        index.insert((slot.user_id, slot.order_id_hi, slot.order_id_lo), handle);
    }
}

#[cfg(test)]
#[path = "wal_test.rs"]
mod wal_test;
