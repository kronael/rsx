//! CMP v4 FAULTED → DXS replay recovery (POC).
//!
//! When `CastReceiver::try_recv` returns `CastRecv::Faulted`,
//! the matching tile must NOT silently advance past lost
//! orders. The recovery path:
//!
//!   1. Open a `ReplicationConsumer` against the risk producer's
//!      DXS server (env: `RSX_ME_REPLAY_DXS_ADDR`).
//!   2. Drain Phase 1 records (seq > `last_delivered_seq`)
//!      until `RECORD_CAUGHT_UP` arrives.
//!   3. Apply each `OrderRequest` / `CancelRequest` to the
//!      in-tile state via [`apply_replayed_record`].
//!   4. Call `cmp_receiver.reset_after_replay(new_tip)` to
//!      resume normal live UDP delivery from `new_tip + 1`.
//!
//! **TODO (downstream re-emit).** Live processing also
//! broadcasts fills + lifecycle events back to risk and
//! marketdata via CMP. This POC intentionally skips that
//! step — downstream consumers must recover their own
//! streams via their own DXS replay paths (per-consumer,
//! future work). The matching tile is internally consistent
//! after `drain_dxs_replay_into_book` returns.
//!
//! **TODO (latency probes).** Live ingestion samples
//! `me_in` / `me_dedup_done` / `me_wal_*` / `me_match_done`.
//! Replay skips these — those probes target live tail
//! latency. Production wiring may want a separate
//! `me_replay` probe set.

use crate::dedup::DedupTracker;
use crate::wal_integration::OrderKey;
use crate::wal_integration::write_events_to_wal;
use crate::wire::OrderMessage;
use rsx_book::book::Orderbook;
use rsx_book::event::CANCEL_USER;
use rsx_book::event::REASON_CANCELLED;
use rsx_book::matching::process_new_order;
use rsx_cast::ReplicationConsumer;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::wal::WalWriter;
use rsx_cast::wal::extract_seq;
use rsx_messages::CancelRequest;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::Qty;
use rsx_types::time::time_ns;
use rustc_hash::FxHashMap;
use std::io;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;

const REASON_DUPLICATE: u8 = 3;

/// Drain a Phase 1 replay from `replay_addr` and apply each
/// record to the local book + dedup + order_index + WAL.
/// Returns the highest seq applied (`new_tip`).
///
/// Stops when `RECORD_CAUGHT_UP` arrives. The caller should
/// pass the result to `CastReceiver::reset_after_replay` to
/// resume normal live UDP delivery.
#[allow(clippy::too_many_arguments)]
pub fn drain_dxs_replay_into_book(
    rt: &tokio::runtime::Runtime,
    book: &mut Orderbook,
    order_index: &mut FxHashMap<OrderKey, u32>,
    dedup: &mut DedupTracker,
    wal_writer: &mut WalWriter,
    symbol_id: u32,
    replay_addr: String,
    last_delivered_seq: u64,
    tip_file: PathBuf,
) -> io::Result<u64> {
    let mut consumer = ReplicationConsumer::from_single(
        symbol_id,
        replay_addr,
        tip_file,
        None,
    )?;
    // Pre-seed tip so the request starts at last_delivered_seq + 1
    // regardless of any stale tip file on disk.
    consumer.tip = last_delivered_seq;

    let mut new_tip = last_delivered_seq;
    let mut applied = 0u64;
    let mut skipped = 0u64;
    let result = rt.block_on(consumer.run_once(
        |raw: RawWalRecord| -> bool {
            if raw.header.record_type == RECORD_CAUGHT_UP {
                return false;
            }
            let seq = extract_seq(&raw.payload).unwrap_or(0);
            if seq <= last_delivered_seq {
                skipped += 1;
                return true;
            }
            if seq > new_tip {
                new_tip = seq;
            }
            apply_replayed_record(
                book,
                order_index,
                dedup,
                wal_writer,
                symbol_id,
                &raw,
            );
            applied += 1;
            true
        },
    ));
    if let Err(e) = result {
        warn!(
            "dxs replay stream ended with error: {e} \
             (applied={applied} skipped={skipped} \
             new_tip={new_tip})",
        );
        return Err(e);
    }
    info!(
        "dxs replay drained: applied={applied} \
         skipped={skipped} new_tip={new_tip}",
    );
    Ok(new_tip)
}

/// Apply a single replayed `OrderRequest` / `CancelRequest`
/// to the in-tile state. Mirrors the live order-handling
/// block in `main` minus the downstream CMP sends — see the
/// module-level TODO.
pub fn apply_replayed_record(
    book: &mut Orderbook,
    order_index: &mut FxHashMap<OrderKey, u32>,
    dedup: &mut DedupTracker,
    wal_writer: &mut WalWriter,
    symbol_id: u32,
    raw: &RawWalRecord,
) {
    match raw.header.record_type {
        RECORD_ORDER_REQUEST
            if raw.payload.len()
                >= std::mem::size_of::<OrderMessage>() =>
        {
            let order_msg = unsafe {
                std::ptr::read_unaligned(
                    raw.payload.as_ptr()
                        as *const OrderMessage,
                )
            };
            let is_dup = dedup.check_and_insert(
                order_msg.user_id,
                order_msg.order_id_hi,
                order_msg.order_id_lo,
            );
            if is_dup {
                let ts = time_ns();
                let mut fail = OrderFailedRecord {
                    seq: 0,
                    ts_ns: ts,
                    user_id: order_msg.user_id,
                    _pad0: 0,
                    order_id_hi: order_msg.order_id_hi,
                    order_id_lo: order_msg.order_id_lo,
                    reason: REASON_DUPLICATE,
                    _pad: [0; 23],
                };
                wal_writer
                    .append(&mut fail)
                    .expect("wal append failed (replay duplicate)");
                return;
            }
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
            wal_writer
                .append(&mut accepted)
                .expect("wal append failed (replay order-accepted)");
            let mut incoming = order_msg.to_incoming();
            process_new_order(book, &mut incoming);
            let ts_ns = time_ns();
            write_events_to_wal(
                wal_writer, book, symbol_id, ts_ns,
            )
            .expect("wal append failed (replay event path)");
            update_order_index(book.events(), order_index);
        }
        RECORD_CANCEL_REQUEST
            if raw.payload.len()
                >= std::mem::size_of::<CancelRequest>() =>
        {
            let req = unsafe {
                std::ptr::read_unaligned(
                    raw.payload.as_ptr()
                        as *const CancelRequest,
                )
            };
            // Mirrors process_cancel in main.rs minus
            // downstream CMP sends. Same WAL/event shape.
            let key: OrderKey = (
                req.user_id,
                req.order_id_hi,
                req.order_id_lo,
            );
            let Some(&handle) = order_index.get(&key) else {
                return;
            };
            let slot_check = book.orders.get(handle);
            if !slot_check.is_active()
                || slot_check.user_id != req.user_id
                || slot_check.order_id_hi != req.order_id_hi
                || slot_check.order_id_lo != req.order_id_lo
            {
                return;
            }
            let remaining_qty = book
                .orders
                .get(handle)
                .remaining_qty;
            book.event_len = 0;
            book.cancel_order(handle);
            book.emit(
                rsx_book::event::Event::OrderCancelled {
                    handle,
                    user_id: req.user_id,
                    remaining_qty,
                    reason: CANCEL_USER,
                    order_id_hi: req.order_id_hi,
                    order_id_lo: req.order_id_lo,
                },
            );
            book.emit(rsx_book::event::Event::OrderDone {
                handle,
                user_id: req.user_id,
                reason: REASON_CANCELLED,
                filled_qty: Qty(0),
                remaining_qty,
                order_id_hi: req.order_id_hi,
                order_id_lo: req.order_id_lo,
            });
            order_index.remove(&key);
            let ts_ns = time_ns();
            write_events_to_wal(
                wal_writer, book, symbol_id, ts_ns,
            )
            .expect("wal append failed (replay cancel path)");
        }
        _ => {
            // Non-input record types (events, BBO, etc.)
            // are produced by ME, not consumed; ignore on
            // replay.
        }
    }
}

/// Keep `order_index` in sync with `book.events()`. Insert
/// on `OrderInserted`, remove on `OrderDone`. Same shape as
/// the local helper in `main.rs`.
fn update_order_index(
    events: &[rsx_book::event::Event],
    index: &mut FxHashMap<OrderKey, u32>,
) {
    for event in events {
        match *event {
            rsx_book::event::Event::OrderInserted {
                handle,
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.insert(
                    (user_id, order_id_hi, order_id_lo),
                    handle,
                );
            }
            rsx_book::event::Event::OrderDone {
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.remove(&(
                    user_id,
                    order_id_hi,
                    order_id_lo,
                ));
            }
            _ => {}
        }
    }
}
