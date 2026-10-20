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

/// Latency sample for `$stage`, measured from the order's gateway timestamp.
macro_rules! stage_sample {
    ($stage:literal, $order:expr) => {
        rsx_log::latency_sample!(
            $stage,
            $order.order_id_hi,
            $order.order_id_lo,
            $order.timestamp_ns
        )
    };
}

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
    // This hop's ingress stamp (specs/2/59-latency-observability.md),
    // carried on every FillRecord this order's match cycle emits.
    let me_in_ns = time_ns();
    stage_sample!("me_in", order_msg);
    let is_dup = dedup.check_and_insert(
        order_msg.user_id,
        order_msg.order_id_hi,
        order_msg.order_id_lo,
    );
    stage_sample!("me_dedup_done", order_msg);

    gauges.orders_processed.fetch_add(1, Ordering::Relaxed);
    if is_dup {
        let mut fail = OrderFailedRecord {
            seq: 0,
            ts_ns: time_ns(),
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
        return;
    }
    let mut accepted = OrderAcceptedRecord {
        seq: 0,
        ts_ns: time_ns(),
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
    stage_sample!("me_wal_accepted_done", order_msg);

    let mut incoming = to_incoming(order_msg);
    process_new_order(book, &mut incoming);
    // Closes the "engine" leg stamped onto every FillRecord, with `me_in_ns`.
    let match_done_ns = time_ns();
    stage_sample!("me_match_done", order_msg);

    // Crash on WAL failure rather than lose fills.
    publish_events(
        wal_writer,
        cast_sender,
        mkt_sender,
        book,
        symbol_id,
        time_ns(),
        order_msg.timestamp_ns,
        order_msg.risk_in_ns,
        me_in_ns,
        match_done_ns,
    )
    .expect("publish_events failed (event path) — violates 6-consistency.md invariant 1 (totally-ordered events) and ordering rule 'Fills precede ORDER_DONE' (§2)");
    gauges.publishes.fetch_add(1, Ordering::Relaxed);
    stage_sample!("me_wal_events_done", order_msg);

    // Keep the (user, oid) -> handle index current for O(1) cancels.
    update_order_index(book.events(), order_index);
    stage_sample!("me_index_done", order_msg);
    stage_sample!("me_out", order_msg);

    // Keep the touch inside the 1:1 zone 0 (eager recenter).
    maybe_recenter(book);
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

#[cfg(test)]
#[path = "order_test.rs"]
mod order_test;
