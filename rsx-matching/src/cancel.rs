use rsx_book::book::Orderbook;
use rsx_book::event::OrderCancelReason;
use rsx_book::event::OrderDoneReason;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalWriter;
use rsx_matching::wal::publish_events;
use rsx_matching::wal::update_order_index;
use rsx_matching::wal::OrderKey;
use rsx_messages::CancelRequest;
use rsx_types::time_utils::time_ns;
use rsx_types::Qty;
use rustc_hash::FxHashMap;
use tracing::warn;

/// Cancel a resting order by `(user, order id)` and publish the events.
///
/// Looks up the slab handle in `order_index` (O(1)) and updates the index
/// from the emitted OrderDone. A miss logs and emits nothing.
pub fn process_cancel(
    book: &mut Orderbook,
    wal_writer: &mut WalWriter,
    cast_sender: &mut CastSender,
    mkt_sender: &mut CastSender,
    order_index: &mut FxHashMap<OrderKey, u32>,
    symbol_id: u32,
    request: &CancelRequest,
) {
    let CancelRequest {
        user_id,
        order_id_hi,
        order_id_lo,
        ..
    } = *request;
    let key: OrderKey = (user_id, order_id_hi, order_id_lo);
    let Some(&found) = order_index.get(&key) else {
        warn!(
            "cancel: order not found \
             user={} id={:#x}/{:#x}",
            user_id, order_id_hi, order_id_lo,
        );
        return;
    };
    // A present key always points at a live slot for this (user, oid): the
    // index is updated on every OrderInserted/OrderDone and rebuilt on
    // restore. Snapshot the pre-cancel qty and BBA before the slot is freed.
    let remaining_qty = book.orders.get(found).remaining_qty;
    let tif = book.orders.get(found).tif;
    let reduce_only = book.orders.get(found).is_reduce_only();
    let old_bid = book.best_bid_tick;
    let old_ask = book.best_ask_tick;
    let old_bid_px = book.best_bid_px;
    let old_ask_px = book.best_ask_px;

    book.event_buf.clear();

    // The checked cancel re-validates the handle in rsx-book. A false
    // return is index/slab drift: trip the debug assert, and in release
    // warn rather than emit cancel events for a slot we did not cancel.
    let cancelled = book.cancel_order_checked(found, user_id, order_id_hi, order_id_lo);
    debug_assert!(
        cancelled,
        "order_index/slab drift: user={} id={:#x}/{:#x} handle={}",
        user_id, order_id_hi, order_id_lo, found,
    );
    if !cancelled {
        warn!(
            "cancel: order_index/slab drift \
             user={} id={:#x}/{:#x} handle={}",
            user_id, order_id_hi, order_id_lo, found,
        );
        return;
    }

    book.emit(rsx_book::event::Event::OrderCancelled {
        handle: found,
        user_id,
        remaining_qty,
        reason: OrderCancelReason::User,
        order_id_hi,
        order_id_lo,
    });
    book.emit(rsx_book::event::Event::OrderDone {
        handle: found,
        tif,
        reduce_only,
        user_id,
        reason: OrderDoneReason::Cancelled,
        filled_qty: Qty(0),
        remaining_qty,
        order_id_hi,
        order_id_lo,
    });

    // Emit BBO if best bid or ask changed (tick OR price). `current_bbo`
    // reads the maintained best px and the side-correct qty at that price
    // — a compressed best level can hold the other side / other prices, so
    // the FIFO head is not the BBO.
    if book.best_bid_tick != old_bid
        || book.best_ask_tick != old_ask
        || book.best_bid_px != old_bid_px
        || book.best_ask_px != old_ask_px
    {
        let bbo = book.current_bbo();
        book.emit(rsx_book::event::Event::BBO {
            bid_px: bbo.bid_px,
            bid_qty: bbo.bid_qty,
            ask_px: bbo.ask_px,
            ask_qty: bbo.ask_qty,
        });
    }

    // No match cycle: no hop timestamps to report.
    publish_events(
        wal_writer, cast_sender, mkt_sender, book, symbol_id, time_ns(), 0, 0, 0, 0,
    )
    .expect("publish_events failed (cancel path) — violates 6-consistency.md invariant 1 (event total order) and invariant 5 (ORDER_DONE commit boundary)");
    update_order_index(book.events(), order_index);
}
