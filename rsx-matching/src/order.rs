use rsx_book::book::Orderbook;
use rsx_book::event::OrderFailureReason;
use rsx_book::matching::process_new_order;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalWriter;
use rsx_health::LoadGauges;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::fan_out;
use rsx_matching::wal::publish_events;
use rsx_matching::wal::update_order_index;
use rsx_matching::wal::OrderKey;
use rsx_matching::wire::to_incoming;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderMessage;
use rsx_types::time_utils::time_ns;
use rsx_types::NONE;
use rustc_hash::FxHashMap;
use std::sync::atomic::Ordering;

/// Handle one RECORD_ORDER_REQUEST: dedup gate, then either the
/// duplicate-reject record or OrderAccepted + match + event publish +
/// order-index update. Runs inside the recv callback.
#[allow(clippy::too_many_arguments)]
pub fn process_order(
    book: &mut Orderbook,
    wal_writer: &mut WalWriter,
    cast_sender: &mut CastSender,
    mkt_sender: &mut CastSender,
    order_index: &mut FxHashMap<OrderKey, u32>,
    dedup: &mut DedupTracker,
    gauges: &LoadGauges,
    symbol_id: u32,
    order_msg: &OrderMessage,
) {
    // specs/2/59-latency-observability.md: this hop's own
    // ingress stamp, carried on every FillRecord this order's
    // match cycle emits.
    let me_in_ns = time_ns();
    // `me_in` records arrival at the matching engine
    // against the gateway submission timestamp.
    rsx_log::latency_sample!(
        "me_in",
        order_msg.order_id_hi,
        order_msg.order_id_lo,
        order_msg.timestamp_ns
    );
    // Dedup check
    let is_dup = dedup.check_and_insert(
        order_msg.user_id,
        order_msg.order_id_hi,
        order_msg.order_id_lo,
    );
    // Sub-stage: dedup completed (after FxHashMap
    // insert). Anchored on the same t0 as me_in.
    rsx_log::latency_sample!(
        "me_dedup_done",
        order_msg.order_id_hi,
        order_msg.order_id_lo,
        order_msg.timestamp_ns
    );

    // Update order counter regardless of dup status.
    gauges.orders_processed.fetch_add(1, Ordering::Relaxed);
    if is_dup {
        let ts = time_ns();
        let mut fail = OrderFailedRecord {
            seq: 0,
            ts_ns: ts,
            user_id: order_msg.user_id,
            _pad0: 0,
            order_id_hi: order_msg.order_id_hi,
            order_id_lo: order_msg.order_id_lo,
            reason: OrderFailureReason::Duplicate as u8,
            _pad: [0; 23],
        };
        fan_out(wal_writer, cast_sender, mkt_sender, &mut fail).expect(
            "wal append failed (duplicate-reject) — violates 6-consistency.md invariant 7 (matching engine persists orderbook via snapshot + WAL)",
        );
    } else {
        let ts = time_ns();
        let mut accepted = OrderAcceptedRecord {
            seq: 0,
            ts_ns: ts,
            user_id: order_msg.user_id,
            symbol_id,
            order_id_hi: order_msg.order_id_hi,
            order_id_lo: order_msg.order_id_lo,
            price: order_msg.price,
            qty: order_msg.qty,
            side: order_msg.side,
            tif: order_msg.tif,
            reduce_only: order_msg.reduce_only,
            post_only: order_msg.post_only,
            cid: [0; 20],
        };
        fan_out(wal_writer, cast_sender, mkt_sender, &mut accepted).expect(
            "wal append failed (order-accepted) — violates 6-consistency.md invariant 7 (WAL persistence) and breaks dedup on replay",
        );
        // Sub-stage: OrderAcceptedRecord appended.
        rsx_log::latency_sample!(
            "me_wal_accepted_done",
            order_msg.order_id_hi,
            order_msg.order_id_lo,
            order_msg.timestamp_ns
        );

        let mut incoming = to_incoming(order_msg);
        process_new_order(book, &mut incoming);
        // specs/2/59-latency-observability.md "engine" leg
        // closer: stamped onto every FillRecord this cycle
        // emits, alongside me_in_ns above.
        let match_done_ns = time_ns();
        // Sub-stage: match cycle finished.
        rsx_log::latency_sample!(
            "me_match_done",
            order_msg.order_id_hi,
            order_msg.order_id_lo,
            order_msg.timestamp_ns
        );

        // Publish events: WAL append + cast send to
        // risk + (selective) cast send to marketdata,
        // each event prepared once (one CRC). Crash
        // on WAL failure rather than lose fills.
        let ts_ns = time_ns();
        publish_events(
                        wal_writer,
                        cast_sender,
                        mkt_sender,
                        book,
                        symbol_id,
                        ts_ns,
                        order_msg.timestamp_ns,
                        order_msg.risk_in_ns,
                        me_in_ns,
                        match_done_ns,
                    )
                    .expect("publish_events failed (event path) — violates 6-consistency.md invariant 1 (totally-ordered events) and ordering rule 'Fills precede ORDER_DONE' (§2)");
        gauges.publishes.fetch_add(1, Ordering::Relaxed);
        // Sub-stage: events flushed + sent.
        rsx_log::latency_sample!(
            "me_wal_events_done",
            order_msg.order_id_hi,
            order_msg.order_id_lo,
            order_msg.timestamp_ns
        );

        // Maintain the (user, oid) -> handle
        // index so subsequent cancels are O(1).
        update_order_index(book.events(), order_index);
        // Sub-stage: order index updated.
        rsx_log::latency_sample!(
            "me_index_done",
            order_msg.order_id_hi,
            order_msg.order_id_lo,
            order_msg.timestamp_ns
        );

        // `me_out` records when ME has finished matching and is
        // about to forward events to risk. t_us
        // measured against the order's gateway
        // submit timestamp. We only log once per
        // incoming order (against its oid).
        rsx_log::latency_sample!(
            "me_out",
            order_msg.order_id_hi,
            order_msg.order_id_lo,
            order_msg.timestamp_ns
        );

        // Keep the touch inside the 1:1 zone 0: recenter the
        // compression map when the mid drifts past half of
        // zone 0. Migration then proceeds lazily
        // (resolve_level per touched price) and on idle
        // cycles (migrate_batch below).
        maybe_recenter(book);
    }
}

/// Recenter the compression map on the current mid when it has drifted
/// past half of zone 0, so the touch stays in the 1:1 zone (strict
/// price-time priority; keeps the compressed-slot slow paths cold).
/// Skips while one-sided (no mid to center on).
///
/// Uses `recenter_now` (eager: swap + full migration in one shot). Lazy
/// per-order migration is NOT correct for live matching — a marketable
/// order's crossing liquidity can sit outside the migrated band, so the
/// ME must never trade against a partially-migrated book. The swap +
/// migration is O(old book size) but recenters are rare (>2.5% drift).
fn maybe_recenter(book: &mut Orderbook) {
    if book.best_bid_tick == NONE || book.best_ask_tick == NONE {
        return;
    }
    let mid = (book.best_bid_px + book.best_ask_px) / 2;
    if book.should_recenter(mid) {
        book.recenter_now(mid);
    }
}
