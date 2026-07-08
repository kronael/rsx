use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::REASON_CANCELLED;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::snapshot;
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
use std::fs;
use std::io;
use std::io::Read as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

/// `(user_id, order_id_hi, order_id_lo) -> slab handle`.
/// Mirror of the type in `main.rs`. Kept here so replay can
/// take ownership of the index without main.rs leaking it.
pub type OrderKey = (u32, u64, u64);

/// `OrderDoneRecord.final_status` is a webproto U-frame status
/// (specs/2/49-webproto.md: 0=FILLED, 2=CANCELLED), NOT a raw matching
/// reason. `REASON_CANCELLED` is 1, which collides with webproto
/// RESTING(1) — a cancelled IOC would surface to the client as "resting"
/// if the raw reason leaked through the gateway. Translate here so
/// `final_status` always holds the webproto status the gateway forwards.
/// OrderDone only ever carries `REASON_FILLED`(0) or `REASON_CANCELLED`(1).
fn done_final_status(reason: u8) -> u8 {
    match reason {
        REASON_CANCELLED => 2, // webproto CANCELLED
        _ => 0,                // REASON_FILLED -> webproto FILLED
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
    emit_events(&mut WalSink { writer }, book, symbol_id, ts_ns, 0, 0)
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
    me_in_ns: u64,
    match_done_ns: u64,
) -> io::Result<()> {
    for event in book.events() {
        match *event {
            Event::Fill {
                maker_handle,
                maker_user_id,
                taker_user_id,
                price,
                qty,
                side,
                maker_order_id_hi,
                maker_order_id_lo,
                taker_order_id_hi,
                taker_order_id_lo,
                gw_in_ns,
            } => {
                let (reduce_only, tif) = if maker_handle != NONE {
                    let slot = book.orders.get(maker_handle);
                    (slot.is_reduce_only() as u8, slot.tif)
                } else {
                    (0, 0)
                };
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
                    reduce_only,
                    tif,
                    post_only: 0,
                    _pad1: [0; 4],
                    gw_in_ns,
                    // Cannot reach this record without growing the
                    // risk->ME wire struct (`OrderMessage`, zero spare
                    // capacity) — see FillRecord's doc comment.
                    risk_in_ns: 0,
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
                    reason,
                    reduce_only,
                    tif,
                    post_only,
                    _pad1: [0; 4],
                };
                sink.emit(&mut record)?;
            }
            Event::OrderDone {
                handle,
                user_id,
                reason,
                filled_qty,
                remaining_qty,
                order_id_hi,
                order_id_lo,
            } => {
                let (reduce_only, tif) = if handle != NONE {
                    let slot = book.orders.get(handle);
                    (slot.is_reduce_only() as u8, slot.tif)
                } else {
                    (0, 0)
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
                    reduce_only,
                    tif,
                    post_only: 0,
                    _pad1: [0; 4],
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
                    reason,
                    _pad: [0; 23],
                };
                // SEQ-1: was WAL-only, which left a seq hole on
                // both live streams every time an order failed at
                // ME. Fan out to both so the seq stays contiguous;
                // the gateway also needs ORDER_FAILED to tell the
                // client its order was rejected.
                sink.emit(&mut record)?;
            }
            Event::BBO {
                bid_px,
                bid_qty,
                ask_px,
                ask_qty,
            } => {
                // SEQ-1 (the bug that caused the FAULTED storms):
                // BBO previously used cmp.send()/mkt.send(), which
                // stamp the CastSender's OWN next_seq — a different
                // counter from the WAL seq used by send_framed for
                // every other record. Since BBO wasn't WAL'd, the
                // two counters desynced and the wire seq regressed
                // → "sender reset detected" → FAULTED. Route BBO
                // through the single WAL seq like every other record.
                // Replay re-derives BBO, so it's a skipped side effect
                // there — but it MUST occupy a WAL seq to keep the
                // stream contiguous (a hole reads as loss → FAULTED).
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
    me_in_ns: u64,
    match_done_ns: u64,
) -> io::Result<()> {
    emit_events(
        &mut FanoutSink { writer, cmp, mkt },
        book,
        symbol_id,
        ts_ns,
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

/// Load book snapshot from
/// `{wal_dir}/{symbol_id}/snapshot.bin`.
/// Returns None if not found or corrupted.
pub fn load_snapshot(wal_dir: &str, symbol_id: u32) -> Option<Box<Orderbook>> {
    let path = PathBuf::from(wal_dir)
        .join(symbol_id.to_string())
        .join("snapshot.bin");
    let mut file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return None,
    };
    match snapshot::load(&mut file) {
        Ok(book) => {
            info!("loaded snapshot from {}", path.display(),);
            Some(book)
        }
        Err(e) => {
            warn!(
                "snapshot load failed: {}, \
                starting empty",
                e,
            );
            None
        }
    }
}

/// Save snapshot + `wal_seq.txt` sidecar via atomic rename. See ARCHITECTURE.md.
pub fn save_snapshot(
    book: &Orderbook,
    wal_dir: &str,
    symbol_id: u32,
    wal_last_seq: u64,
) -> io::Result<()> {
    let dir = PathBuf::from(wal_dir).join(symbol_id.to_string());
    let tmp = dir.join("snapshot.bin.tmp");
    let dest = dir.join("snapshot.bin");
    let mut file = fs::File::create(&tmp)?;
    snapshot::save(book, &mut file)?;
    file.sync_all()?;
    fs::rename(&tmp, &dest)?;

    let seq_tmp = dir.join("wal_seq.txt.tmp");
    let seq_dest = dir.join("wal_seq.txt");
    let mut seq_file = fs::File::create(&seq_tmp)?;
    write!(seq_file, "{}", wal_last_seq)?;
    seq_file.sync_all()?;
    fs::rename(&seq_tmp, &seq_dest)?;
    Ok(())
}

/// Load the WAL seq sidecar written by [`save_snapshot`].
/// Returns `None` if missing or unparseable; the caller must
/// then fall back to "no snapshot" (full replay from seq 1).
pub fn load_wal_seq(wal_dir: &str, symbol_id: u32) -> Option<u64> {
    let path = PathBuf::from(wal_dir)
        .join(symbol_id.to_string())
        .join("wal_seq.txt");
    let mut s = String::new();
    fs::File::open(&path).ok()?.read_to_string(&mut s).ok()?;
    s.trim().parse::<u64>().ok()
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
/// Covers both pre- and post-snapshot records and is idempotent (a set).
/// (bugs.md ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD, dedup half)
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
