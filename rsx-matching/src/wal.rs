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

/// Write all events from the book's event buffer to WAL.
pub fn write_events_to_wal(
    writer: &mut WalWriter,
    book: &Orderbook,
    symbol_id: u32,
    ts_ns: u64,
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
                taker_ts_ns,
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
                    taker_ts_ns,
                };
                {
                    let framed = writer.prepare(&mut record)?;
                    writer.append_framed(&framed)?;
                }
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
                {
                    let framed = writer.prepare(&mut record)?;
                    writer.append_framed(&framed)?;
                }
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
                {
                    let framed = writer.prepare(&mut record)?;
                    writer.append_framed(&framed)?;
                }
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
                {
                    let framed = writer.prepare(&mut record)?;
                    writer.append_framed(&framed)?;
                }
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
                {
                    let framed = writer.prepare(&mut record)?;
                    writer.append_framed(&framed)?;
                }
            }
            Event::BBO { .. } => {
                // BBO not persisted to WAL (derived on replay)
            }
        }
    }
    Ok(())
}

/// Publish each event once (WAL prepare = single CRC + seq), then fan out to WAL +
/// risk-bound + (selectively) marketdata-bound `CastSender`. See ARCHITECTURE.md.
pub fn publish_events(
    writer: &mut WalWriter,
    cmp: &mut CastSender,
    mkt: &mut CastSender,
    book: &Orderbook,
    symbol_id: u32,
    ts_ns: u64,
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
                taker_ts_ns,
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
                    taker_ts_ns,
                };
                fan_out(writer, cmp, Some(mkt), &mut record)?;
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
                fan_out(writer, cmp, Some(mkt), &mut record)?;
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
                fan_out(writer, cmp, Some(mkt), &mut record)?;
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
                fan_out(writer, cmp, Some(mkt), &mut record)?;
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
                fan_out(writer, cmp, Some(mkt), &mut record)?;
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
                // through fan_out on the single WAL seq like every
                // other record. It is now WAL'd (cheap; replay
                // skips it since BBO is re-derived) so live seq ==
                // replay seq and both streams stay contiguous.
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
                fan_out(writer, cmp, Some(mkt), &mut record)?;
            }
        }
    }
    Ok(())
}

/// Frame once via `WalWriter::prepare`, then fan to WAL + cmp + optional mkt.
#[inline]
fn fan_out<T: rsx_cast::CastRecord>(
    writer: &mut WalWriter,
    cmp: &mut CastSender,
    mkt: Option<&mut CastSender>,
    record: &mut T,
) -> io::Result<()> {
    let framed: Framed = writer.prepare(record)?;
    writer.append_framed(&framed)?;
    cmp.send_framed(&framed)?;
    if let Some(s) = mkt {
        s.send_framed(&framed)?;
    }
    Ok(())
}

/// Flush WAL if 10ms have elapsed since last flush.
#[inline]
pub fn flush_if_due(writer: &mut WalWriter, last_flush: &mut Instant) -> io::Result<()> {
    let now = Instant::now();
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
    dedup: &mut crate::dedup::DedupTracker,
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
                // Re-record dedup so a duplicate that arrives
                // post-restart is still rejected.
                let _ = dedup.check_and_insert(rec.user_id, rec.order_id_hi, rec.order_id_lo);
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
                update_order_index_local(book.events(), order_index);
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

/// Local copy of main.rs's `update_order_index` so replay
/// doesn't need to import a private symbol. Keeps the same
/// shape: a successful insert/restore puts the handle into
/// the index; a Done removes it.
fn update_order_index_local(events: &[Event], index: &mut FxHashMap<OrderKey, u32>) {
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
