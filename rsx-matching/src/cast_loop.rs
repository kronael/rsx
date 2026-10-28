//! The matching loop: one pinned thread draining the risk cast
//! receiver straight into the book, writing the WAL and fanning events
//! out to risk and marketdata. Std non-blocking UDP busy-spin; no async
//! runtime on this thread.

use crate::cancel::process_cancel;
use crate::order::process_order;
use rsx_book::book::Orderbook;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::decode_payload;
use rsx_cast::wal::WalWriter;
use rsx_matching::checkpoint::CheckpointWriter;
use rsx_matching::checkpoint::MatchingCheckpointV1;
use rsx_matching::config::apply_due_configs;
use rsx_matching::config::ConfigLoopEnd;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::metrics::MatchingMetrics;
use rsx_matching::wal::flush_if_due;
use rsx_matching::wal::CastPeer;
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
/// recovery. `config_link` is `None` when config polling is disabled;
/// otherwise the config sidecar feeds it due configs.
pub struct LoopState {
    pub book: Orderbook,
    pub wal_writer: WalWriter,
    pub dedup: DedupTracker,
    pub order_index: FxHashMap<OrderKey, u32>,
    pub cast_receiver: CastReceiver,
    pub cast_sender: CastPeer,
    pub mkt_sender: CastPeer,
    pub metrics: Arc<MatchingMetrics>,
    pub checkpoint_writer: CheckpointWriter,
    pub symbol_id: u32,
    pub config_link: Option<ConfigLoopEnd>,
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
        metrics,
        checkpoint_writer,
        symbol_id,
        mut config_link,
    } = state;

    // Cached monotonic clock: sampled once every CLOCK_REFRESH_SPINS loop
    // iterations and reused for dedup pruning + the flush/snapshot
    // timers, so the hot loop stops paying ~4 `Instant::now()` per spin.
    // The hour-scale dedup window and 10 ms flush tolerate the coarse tick.
    const CLOCK_REFRESH_SPINS: u32 = 1024;
    let mut clock = Instant::now();
    let mut last_flush = clock;
    let mut last_snapshot = clock;
    let mut checkpoint_due = false;
    let mut last_user_maintenance = clock;
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
            let final_checkpoint = MatchingCheckpointV1::capture(&book, wal_writer.last_seq())
                .map_err(|error| warn!("shutdown snapshot capture failed: {error}"))
                .ok();
            if let Err(error) = checkpoint_writer.shutdown(final_checkpoint) {
                warn!("shutdown checkpoint writer failed: {error}");
            }
            info!("matching engine shutdown complete");
            std::process::exit(0);
        }
        // Zero-copy: orders and cancels from Risk are handled inside the
        // callback, borrowing the receiver's recv buffer.
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
                        &metrics,
                        symbol_id,
                        &order_msg,
                    );
                } else {
                    warn!("order request too short, dropped: {} bytes", payload.len());
                }
            } else if hdr.record_type == RECORD_CANCEL_REQUEST {
                if let Some(req) = decode_payload::<CancelRequest>(payload) {
                    process_cancel(
                        &mut book,
                        &mut wal_writer,
                        &mut cast_sender,
                        &mut mkt_sender,
                        &mut order_index,
                        symbol_id,
                        &req,
                    );
                } else {
                    warn!("cancel request too short, dropped: {} bytes", payload.len());
                }
            } else {
                warn!(
                    "unknown record type {} dropped: {} bytes",
                    hdr.record_type,
                    payload.len()
                );
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
            metrics.drops.add(skipped);
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
        // Idle tick (Empty/Reconnect): data was handled in the callback.
        if !matches!(recv, CastRecvWith::Data)
            && clock.duration_since(last_user_maintenance).as_millis() >= 1
        {
            book.maintain_users(time_ns(), false);
            last_user_maintenance = clock;
        }

        dedup.maybe_cleanup();
        metrics.dedup_entries.set(dedup.len() as u64);
        metrics.wal_last_seq.set(wal_writer.last_seq());

        // Configs the sidecar polled from Postgres; ring pops only.
        if let Some(link) = config_link.as_mut() {
            if let Some(config_version) = apply_due_configs(
                link,
                &mut book,
                &mut wal_writer,
                &mut cast_sender,
                &mut mkt_sender,
                symbol_id,
            ) {
                metrics.config_version.set(config_version);
                checkpoint_due = true;
            }
        }

        if flush_if_due(&mut wal_writer, &mut last_flush, clock) {
            metrics.wal_flushed_seq.set(wal_writer.last_seq());
        }

        // Capture briefly on the ME thread; disk work stays off-thread. A
        // config apply makes it due at once: recovery trusts the checkpoint's
        // tick and lot.
        if checkpoint_due || clock.duration_since(last_snapshot).as_secs() >= 10 {
            if let Err(error) = MatchingCheckpointV1::capture(&book, wal_writer.last_seq())
                .and_then(|checkpoint| checkpoint_writer.submit(checkpoint))
            {
                warn!("checkpoint capture or submit failed: {error}");
            }
            checkpoint_due = false;
            last_snapshot = clock;
        }

        cast_sender.tick();
        mkt_sender.tick();
        cast_sender.sender.recv_control();
        mkt_sender.sender.recv_control();
    }
}
