//! The matching loop: one pinned thread draining the risk cast
//! receiver straight into the book, writing the WAL and fanning events
//! out to risk and marketdata. Std non-blocking UDP busy-spin; no async
//! runtime on this thread.

use crate::cancel::process_cancel;
use crate::order::process_order;
use rsx_book::book::Orderbook;
use rsx_book::snapshot;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::WalWriter;
use rsx_health::LoadGauges;
use rsx_matching::checkpoint::CheckpointWriter;
use rsx_matching::checkpoint::MatchingCheckpointV1;
use rsx_matching::config::apply_scheduled_configs;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::flush_if_due;
use rsx_matching::wal::update_order_index;
use rsx_matching::wal::OrderKey;
use rsx_messages::CancelRequest;
use rsx_messages::OrderMessage;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::time_utils::time_ns;
use rustc_hash::FxHashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tracing::error;
use tracing::info;
use tracing::warn;

/// Everything the loop owns, wired by the composition root after
/// recovery. `pg_client` is `None` when config polling is disabled;
/// `rt` drives its queries.
pub struct LoopState {
    pub book: Orderbook,
    pub wal_writer: WalWriter,
    pub dedup: DedupTracker,
    pub order_index: FxHashMap<OrderKey, u32>,
    pub cast_receiver: CastReceiver,
    pub cast_sender: CastSender,
    pub mkt_sender: CastSender,
    pub gauges: Arc<LoadGauges>,
    pub checkpoint_writer: CheckpointWriter,
    pub symbol_id: u32,
    pub pg_client: Option<tokio_postgres::Client>,
    pub rt: tokio::runtime::Runtime,
    pub config_version: u64,
    pub last_flush: Instant,
    pub last_snapshot: Instant,
    pub last_config_poll: Instant,
    pub last_user_maintenance: Instant,
}

/// Busy-spin until the shutdown flag flips, then flush the WAL, write
/// a final checkpoint and exit the process.
pub fn run(state: LoopState, shutdown: &AtomicBool) -> ! {
    let LoopState {
        mut book,
        mut wal_writer,
        mut dedup,
        mut order_index,
        mut cast_receiver,
        mut cast_sender,
        mut mkt_sender,
        gauges,
        checkpoint_writer,
        symbol_id,
        pg_client,
        rt,
        mut config_version,
        mut last_flush,
        mut last_snapshot,
        mut last_config_poll,
        mut last_user_maintenance,
    } = state;

    // Cached monotonic clock: sampled once every CLOCK_REFRESH_SPINS loop
    // iterations and reused for dedup pruning + the flush/snapshot/config
    // timers, so the hot loop stops paying ~4 `Instant::now()` per spin.
    // The hour-scale dedup window and 10 ms flush tolerate the coarse tick.
    const CLOCK_REFRESH_SPINS: u32 = 1024;
    let mut clock = Instant::now();
    let mut spin_count: u32 = 0;

    loop {
        if spin_count.is_multiple_of(CLOCK_REFRESH_SPINS) {
            clock = Instant::now();
        }
        spin_count = spin_count.wrapping_add(1);
        dedup.refresh_clock(clock);

        if shutdown.load(Ordering::SeqCst) {
            info!("shutdown signal received, draining wal");
            if let Err(e) = wal_writer.flush() {
                error!("shutdown wal flush failed: {e}");
            }
            let final_checkpoint = match snapshot::capture(&book) {
                Ok(book_snapshot) => Some(MatchingCheckpointV1::new(
                    wal_writer.last_seq(),
                    book_snapshot,
                )),
                Err(error) => {
                    warn!("shutdown snapshot capture failed: {error}");
                    None
                }
            };
            if let Err(error) = checkpoint_writer.shutdown(final_checkpoint) {
                warn!("shutdown checkpoint writer failed: {error}");
            }
            info!("matching engine shutdown complete");
            std::process::exit(0);
        }
        // Receive orders/cancels via casting/UDP from Risk.
        // Zero-copy: the order-processing body runs inside the
        // callback, borrowing the receiver's recv buffer — no
        // per-message Vec allocation. The closure cannot
        // `continue`/`break` the outer loop, but the Data body
        // never needs to (the dup path is an if/else, not an
        // early return); Faulted/Empty/Reconnect are handled in
        // the match below where `continue` is legal.
        let recv = cast_receiver.try_recv_with(|hdr, payload| {
            if hdr.record_type == RECORD_ORDER_REQUEST {
                if let Some(order_msg) = decode_payload::<OrderMessage>(payload) {
                    process_order(
                        &mut book,
                        &mut wal_writer,
                        &mut cast_sender,
                        &mut mkt_sender,
                        &mut order_index,
                        &mut dedup,
                        &gauges,
                        symbol_id,
                        &order_msg,
                    );
                }
            } else if hdr.record_type == RECORD_CANCEL_REQUEST {
                if let Some(req) = decode_payload::<CancelRequest>(payload) {
                    process_cancel(
                        &mut book,
                        &mut wal_writer,
                        &mut cast_sender,
                        &mut mkt_sender,
                        &order_index,
                        symbol_id,
                        req.user_id,
                        req.order_id_hi,
                        req.order_id_lo,
                    );
                    update_order_index(book.events(), &mut order_index);
                }
            }
        });
        if let CastRecvWith::Faulted {
            last_delivered_seq,
            gap_start,
            gap_end_inclusive,
        } = recv
        {
            // Drop-safe skip. FAULTED = an unrecoverable gap in the
            // risk->ME order stream. That stream is recovered at the
            // APPLICATION layer, not the transport: a dropped pre-ack
            // order is re-sent by the client (no-ack-within-timeout,
            // spec 49-webproto) and deduped on the ME's WAL
            // (RECORD_ORDER_ACCEPTED) => exactly-once. The ME
            // re-sequences on output (its own WAL seq), so an inbound
            // gap is NOT an output gap — risk/recorder/marketdata still
            // see a contiguous ME stream. So skip the gap and resume
            // live rather than replay-or-panic. (ME's WAL replication
            // SERVER, RSX_ME_REPLICATION_BIND_ADDR, still lets RISK
            // recover FILL delivery — a different, still-required path.)
            let skipped = gap_end_inclusive.saturating_sub(gap_start) + 1;
            gauges.drops.fetch_add(skipped, Ordering::Relaxed);
            warn!(
                "matching loop FAULTED: skipping unrecoverable order \
                 gap [{}..={}] ({} seq) after last_delivered={}; \
                 clients re-send dropped pre-ack orders (WAL dedup = \
                 exactly-once)",
                gap_start, gap_end_inclusive, skipped, last_delivered_seq,
            );
            cast_receiver.reset_after_replay(gap_end_inclusive);
            continue;
        }
        // Nothing delivered this tick (Empty/Reconnect): make
        // progress on a background compression-map migration if
        // one is in flight. Data was already processed in the
        // callback above.
        if !matches!(recv, CastRecvWith::Data) && book.is_migrating() {
            book.migrate_batch(100);
        }
        if !matches!(recv, CastRecvWith::Data)
            && clock.duration_since(last_user_maintenance).as_millis() >= 1
        {
            book.maintain_users(time_ns(), false);
            last_user_maintenance = clock;
        }

        dedup.maybe_cleanup();
        gauges
            .dedup_map_size
            .store(dedup.len() as u64, Ordering::Relaxed);

        // Poll config every 10 minutes
        if clock.duration_since(last_config_poll).as_secs() >= 600 {
            if let Some(ref client) = pg_client {
                apply_scheduled_configs(
                    &rt,
                    client,
                    &mut book,
                    &mut wal_writer,
                    &mut cast_sender,
                    &mut mkt_sender,
                    symbol_id,
                    &mut config_version,
                );
            }
            last_config_poll = clock;
        }

        if let Err(e) = flush_if_due(&mut wal_writer, &mut last_flush, clock) {
            warn!("matching: wal flush_if_due failed: {e}");
        }

        // Capture briefly on the ME thread; disk work stays off-thread.
        if clock.duration_since(last_snapshot).as_secs() >= 10 {
            match snapshot::capture(&book) {
                Ok(book_snapshot) => {
                    let checkpoint =
                        MatchingCheckpointV1::new(wal_writer.last_seq(), book_snapshot);
                    if let Err(error) = checkpoint_writer.submit(checkpoint) {
                        warn!("checkpoint submit failed: {error}");
                    }
                }
                Err(error) => {
                    warn!("checkpoint capture failed: {error}");
                }
            }
            last_snapshot = clock;
        }

        if let Err(e) = cast_sender.tick() {
            warn!("cast_sender tick (heartbeat) failed: {e}");
        }
        if let Err(e) = mkt_sender.tick() {
            warn!("mkt_sender tick (heartbeat) failed: {e}");
        }
        cast_sender.recv_control();
        mkt_sender.recv_control();
    }
}
