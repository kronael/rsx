use crate::recovery::RecoveryError;
use crate::wire::decode_side;
use crate::wire::decode_tif;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderDoneReason;
use rsx_book::event::OrderFailureReason;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::Framed;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_cast::CastRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::CANCEL_REASON_EXPIRY;
use rsx_messages::CANCEL_REASON_POST_ONLY_REJECT;
use rsx_messages::CANCEL_REASON_SYSTEM;
use rsx_messages::CANCEL_REASON_USER_CANCEL;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::FailureReason;
use rsx_types::NONE;
use rustc_hash::FxHashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

/// `(user_id, order_id_hi, order_id_lo) -> slab handle`.
pub type OrderKey = (u32, u64, u64);

/// Translate the book's done reason into `(final_status, cancel_reason)`:
/// status 0 is FILLED and 2 is CANCELLED, the reason a wire
/// `CANCEL_REASON_*` code (specs/2/18-messages.md). An IOC remainder is a
/// time-in-force expiry (specs/2/10); a residual cut off by the per-order
/// maker cap is a system cancel.
fn done_status_and_reason(reason: OrderDoneReason) -> (u8, u8) {
    match reason {
        OrderDoneReason::Filled => (0, 0),
        OrderDoneReason::UserCancel => (2, CANCEL_REASON_USER_CANCEL),
        OrderDoneReason::Expired => (2, CANCEL_REASON_EXPIRY),
        OrderDoneReason::PostOnly => (2, CANCEL_REASON_POST_ONLY_REJECT),
        OrderDoneReason::MatchLimit => (2, CANCEL_REASON_SYSTEM),
    }
}

/// Translate the book's failure reason into the wire `FailureReason` code
/// (specs/2/18-messages.md). The book's discriminants are not the wire
/// values; the gateway forwards the byte and clients decode the wire table.
pub fn failure_reason_code(reason: OrderFailureReason) -> u8 {
    let wire = match reason {
        OrderFailureReason::InvalidTick => FailureReason::InvalidTickSize,
        OrderFailureReason::InvalidLot => FailureReason::InvalidLotSize,
        OrderFailureReason::ReduceOnly => FailureReason::ReduceOnlyViolation,
        OrderFailureReason::FillOrKill => FailureReason::FillOrKill,
        OrderFailureReason::Duplicate => FailureReason::DuplicateOrderId,
    };
    wire as u8
}

/// `(reduce_only, tif)` wire bytes of the slot at `handle`; zeros for `NONE`.
fn slot_flags(book: &Orderbook, handle: u32) -> (u8, u8) {
    if handle == NONE {
        return (0, 0);
    }
    let slot = book.orders.get(handle);
    (slot.is_reduce_only() as u8, slot.tif)
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
                    taker_fee_bps: 0,
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
                    maker_fee_bps: 0,
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
                let (reduce_only, tif) = slot_flags(book, handle);
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
                order_id_hi,
                order_id_lo,
            } => {
                let (reduce_only, tif) = slot_flags(book, handle);
                let mut record = OrderCancelledRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    remaining_qty,
                    reason: CANCEL_REASON_USER_CANCEL,
                    reduce_only,
                    tif,
                    post_only: 0,
                    _pad1: [0; 4],
                    _pad2: [0; 8],
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
                let (final_status, cancel_reason) = done_status_and_reason(reason);
                let mut record = OrderDoneRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    filled_qty,
                    remaining_qty,
                    final_status,
                    reduce_only: reduce_only as u8,
                    tif,
                    post_only: 0,
                    cancel_reason,
                    _pad1: [0; 3],
                    gw_in_ns,
                    risk_in_ns,
                    me_in_ns,
                    match_done_ns,
                    gw_out_ns: 0,
                    _pad2: [0; 24],
                };
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
                    reason: failure_reason_code(reason),
                    _pad: [0; 23],
                };
                sink.emit(&mut record)?;
            }
            Event::BBO {
                bid_px,
                bid_qty,
                ask_px,
                ask_qty,
            } => {
                // Replay re-derives BBO, but it still takes a WAL seq so
                // both live streams stay contiguous.
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
                    _pad3: [0; 56],
                };
                sink.emit(&mut record)?;
            }
        }
    }
    Ok(())
}

/// Publish each event once (WAL prepare = single CRC + seq), then fan out to
/// WAL + risk + marketdata. Production path. See ARCHITECTURE.md. The hop
/// timestamps are as for `emit_events`; the cancel path passes `0`.
#[allow(clippy::too_many_arguments)]
pub fn publish_events(
    writer: &mut WalWriter,
    cmp: &mut CastPeer,
    mkt: &mut CastPeer,
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
    fn emit<T: CastRecord>(&mut self, record: &mut T) -> io::Result<()>;
}

/// WAL-only sink: prepare (assign seq + CRC), then append. Replay/bench helper.
struct WalSink<'a> {
    writer: &'a mut WalWriter,
}

impl EventSink for WalSink<'_> {
    fn emit<T: CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        let framed = self.writer.prepare(record)?;
        self.writer.append_framed(&framed)
    }
}

/// Production sink: frame once (single CRC + seq), then fan out to WAL + risk
/// + marketdata. See ARCHITECTURE.md.
struct FanoutSink<'a> {
    writer: &'a mut WalWriter,
    cmp: &'a mut CastPeer,
    mkt: &'a mut CastPeer,
}

impl EventSink for FanoutSink<'_> {
    fn emit<T: CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        fan_out(self.writer, self.cmp, self.mkt, record)
    }
}

/// A cast stream to one downstream peer (risk or marketdata) and whether its
/// sends are currently failing. A peer that is down fails every send, so the
/// failure is logged once per outage episode, not once per record
/// (the `persist_stalled` shape of `rsx-risk`).
pub struct CastPeer {
    pub sender: CastSender,
    name: &'static str,
    failing: bool,
    heartbeat_failing: bool,
}

impl CastPeer {
    pub fn new(name: &'static str, sender: CastSender) -> Self {
        Self {
            sender,
            name,
            failing: false,
            heartbeat_failing: false,
        }
    }

    /// Whether the last send failed.
    pub fn failing(&self) -> bool {
        self.failing
    }

    /// Heartbeat the peer when one is due. A failure warns on the first tick
    /// of an episode and stays silent until a heartbeat goes out again; the
    /// loop calls this every spin.
    pub fn tick(&mut self) {
        match self.sender.tick() {
            Ok(()) if self.heartbeat_failing => {
                info!("{} heartbeat recovered", self.name);
                self.heartbeat_failing = false;
            }
            Ok(()) => {}
            Err(error) if !self.heartbeat_failing => {
                warn!(
                    "{} heartbeat failed, quiet until it recovers: {error}",
                    self.name
                );
                self.heartbeat_failing = true;
            }
            Err(_) => {}
        }
    }

    /// Send `framed`. A failure warns on the first send of an episode and
    /// stays silent until a send succeeds again; either way the caller
    /// carries on, since the WAL already holds the record.
    fn send(&mut self, framed: &Framed, record_type: u16) {
        match self.sender.send_framed(framed) {
            Ok(()) if self.failing => {
                info!("{} send recovered", self.name);
                self.failing = false;
            }
            Ok(()) => {}
            Err(error) if !self.failing => {
                warn!(
                    "{} send failed (type={record_type}), quiet until it recovers: {error}",
                    self.name
                );
                self.failing = true;
            }
            Err(_) => {}
        }
    }
}

/// The one path every WAL-framed record takes: frame once (single seq +
/// CRC), append to the WAL, then send the same frame to risk and
/// marketdata. Every record that consumes a WAL seq goes to both streams
/// so neither has a hole. A WAL error is returned for the caller to treat
/// as fatal; a failed cast send only warns (once per outage, see
/// [`CastPeer`]), and the other send still runs. A full WAL buffer stalls
/// the caller until a flush frees it (`specs/2/48-wal.md`); it is not an
/// error.
pub fn fan_out<T: CastRecord>(
    writer: &mut WalWriter,
    cmp: &mut CastPeer,
    mkt: &mut CastPeer,
    record: &mut T,
) -> io::Result<()> {
    let framed: Framed = writer.prepare(record)?;
    append_stalling(writer, &framed)?;
    cmp.send(&framed, T::record_type());
    mkt.send(&framed, T::record_type());
    Ok(())
}

/// Append `framed`, flushing first whenever the writer's buffer is full.
/// Any other append error is returned.
fn append_stalling(writer: &mut WalWriter, framed: &Framed) -> io::Result<()> {
    loop {
        match writer.append_framed(framed) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                warn!("wal buffer full, stalling until flushed");
                flush_stalling(writer);
            }
            result => return result,
        }
    }
}

/// Flush the WAL, retrying until the flush succeeds.
fn flush_stalling(writer: &mut WalWriter) {
    retry_until_ok("wal flush", || writer.flush());
}

/// Run `operation` until it succeeds, sleeping 1 ms between attempts and
/// warning on the first failure only. Blocks the calling thread; this is the
/// producer stall of `specs/2/48-wal.md` when the disk cannot keep up.
fn retry_until_ok(what: &str, mut operation: impl FnMut() -> io::Result<()>) {
    let mut failed = false;
    while let Err(error) = operation() {
        if !failed {
            warn!("{what} failed, stalling until it succeeds: {error}");
            failed = true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Flush the WAL when 10ms have elapsed since the last flush or
/// `should_flush` reports the record threshold, retrying a failed flush
/// until it succeeds. Returns whether it flushed. `now` is the caller's
/// cached loop clock, so the check costs no `Instant::now()`.
pub fn flush_if_due(writer: &mut WalWriter, last_flush: &mut Instant, now: Instant) -> bool {
    if writer.should_flush() || now.duration_since(*last_flush).as_millis() >= 10 {
        flush_stalling(writer);
        *last_flush = now;
        return true;
    }
    false
}

/// Decode `raw` as `T`. A payload shorter than `T` has a valid CRC but
/// cannot be a record of this type, so it is corruption or a framing bug.
fn decode_record<T: Copy>(raw: &RawWalRecord) -> Result<T, RecoveryError> {
    decode_payload::<T>(&raw.payload).ok_or_else(|| RecoveryError::UndecodableRecord {
        record_type: raw.header.record_type,
        seq: extract_seq(&raw.payload),
        payload_len: raw.payload.len(),
        expected_len: std::mem::size_of::<T>(),
    })
}

/// Record types the ME itself writes whose replay has no book effect: the
/// events regenerated by re-executing `OrderAccepted`.
fn is_replay_side_effect(record_type: u16) -> bool {
    matches!(
        record_type,
        RECORD_FILL
            | RECORD_BBO
            | RECORD_ORDER_INSERTED
            | RECORD_ORDER_DONE
            | RECORD_ORDER_FAILED
            | RECORD_CONFIG_APPLIED
    )
}

/// Replay WAL records after a snapshot to bring the book to current state.
/// Returns the highest WAL seq read. A record of a type replay applies whose
/// payload does not decode is an error, and so is any read failure; unknown
/// record types are skipped with one warning per type
/// (`specs/2/10-replication.md` § unknown record types). See ARCHITECTURE.md.
pub fn replay_wal_after_snapshot(
    book: &mut Orderbook,
    order_index: &mut FxHashMap<OrderKey, u32>,
    wal_dir: &str,
    symbol_id: u32,
    start_seq: u64,
) -> Result<u64, RecoveryError> {
    let wal_path = PathBuf::from(wal_dir);
    let read_error = |source| RecoveryError::ReplayRead { start_seq, source };
    let mut reader =
        WalReader::open_from_seq(symbol_id, start_seq, &wal_path).map_err(read_error)?;
    let mut last_seq = start_seq.saturating_sub(1);
    let mut accepted = 0u64;
    let mut cancelled = 0u64;
    let mut unknown_types: Vec<u16> = Vec::new();
    while let Some(raw) = reader.next().map_err(read_error)? {
        if let Some(seq) = extract_seq(&raw.payload) {
            last_seq = last_seq.max(seq);
        }
        match raw.header.record_type {
            RECORD_ORDER_ACCEPTED => {
                let rec = decode_record::<OrderAcceptedRecord>(&raw)?;
                // Dedup is NOT re-seeded here — `rebuild_dedup_window`
                // owns the whole 1 h window (pre- AND post-snapshot) in
                // one ascending-seq pass, which keeps the pruning queue
                // ordered. This forward pass only reconstructs book + index.
                let mut incoming = IncomingOrder {
                    price: rec.price,
                    qty: rec.qty,
                    remaining_qty: rec.qty,
                    side: decode_side(rec.side),
                    tif: decode_tif(rec.tif),
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
            RECORD_ORDER_CANCELLED => {
                let rec = decode_record::<OrderCancelledRecord>(&raw)?;
                let key: OrderKey = (rec.user_id, rec.order_id_hi, rec.order_id_lo);
                if let Some(&handle) = order_index.get(&key) {
                    book.cancel_order(handle);
                    order_index.remove(&key);
                    cancelled += 1;
                }
            }
            record_type if is_replay_side_effect(record_type) => {}
            record_type => {
                if !unknown_types.contains(&record_type) {
                    unknown_types.push(record_type);
                    warn!("wal replay: skipping unknown record type {record_type}");
                }
            }
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
/// A checkpoint persists the slab but not the dedup set, and the checkpoint
/// cadence (~10 s) is far shorter than the window (`DEDUP_WINDOW`, 1 h), so
/// a resend of an order accepted before the checkpoint would otherwise
/// execute twice. Scan every retained `RECORD_ORDER_ACCEPTED` in seq order
/// and `seed` each key still inside the window with its remaining TTL
/// (record `ts_ns` vs `now_unix_ns`); seq order feeds `seed` oldest-first,
/// keeping the pruning queue ordered. Idempotent. An `ORDER_ACCEPTED` whose
/// payload does not decode is an error: skipping it would leave its key
/// unprotected.
pub fn rebuild_dedup_window(
    dedup: &mut crate::dedup::DedupTracker,
    wal_dir: &str,
    symbol_id: u32,
    now_unix_ns: u64,
) -> Result<u64, RecoveryError> {
    let wal_path = PathBuf::from(wal_dir);
    let read_error = |source| RecoveryError::DedupRead { source };
    // Filter by ts_ns, not seq, so start from the earliest retained file.
    let mut reader = WalReader::open_from_seq(symbol_id, 0, &wal_path).map_err(read_error)?;
    let before = dedup.len();
    while let Some(raw) = reader.next().map_err(read_error)? {
        if raw.header.record_type != RECORD_ORDER_ACCEPTED {
            continue;
        }
        let rec = decode_record::<OrderAcceptedRecord>(&raw)?;
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

/// The one order-index maintainer, shared by the live loop, replay and the
/// bench harness: an insert puts the handle into the index; a Done removes it.
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
