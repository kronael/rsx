use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_dxs::wal::WalWriter;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::PayloadPreamble;
use std::io;
use std::time::Instant;

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
                maker_handle: _,
                maker_user_id,
                taker_user_id,
                price,
                qty,
                side,
                maker_order_id_hi,
                maker_order_id_lo,
                taker_order_id_hi,
                taker_order_id_lo,
            } => {
                let record = FillRecord {
                    preamble: make_prefix::<FillRecord>(),
                    ts_ns,
                    symbol_id,
                    taker_user_id,
                    maker_user_id,
                    _pad0: 0,
                    taker_order_id_hi,
                    taker_order_id_lo,
                    maker_order_id_hi,
                    maker_order_id_lo,
                    price: price.0,
                    qty: qty.0,
                    taker_side: side,
                    reduce_only: 0,
                    tif: 0,
                    post_only: 0,
                    _pad1: [0; 4],
                };
                let bytes = record_as_bytes(&record);
                writer.append(RECORD_FILL, bytes)?;
            }
            Event::OrderInserted {
                handle: _,
                user_id,
                side,
                price,
                qty,
                order_id_hi,
                order_id_lo,
            } => {
                let record = OrderInsertedRecord {
                    preamble: make_prefix::<OrderInsertedRecord>(),
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    price: price.0,
                    qty: qty.0,
                    side,
                    reduce_only: 0,
                    tif: 0,
                    post_only: 0,
                    _pad1: [0; 4],
                };
                let bytes = record_as_bytes(&record);
                writer.append(
                    RECORD_ORDER_INSERTED, bytes,
                )?;
            }
            Event::OrderCancelled {
                handle: _,
                user_id,
                remaining_qty,
                order_id_hi,
                order_id_lo,
            } => {
                let record = OrderCancelledRecord {
                    preamble: make_prefix::<OrderCancelledRecord>(),
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    remaining_qty: remaining_qty.0,
                    reason: 1,
                    reduce_only: 0,
                    tif: 0,
                    post_only: 0,
                    _pad1: [0; 4],
                };
                let bytes = record_as_bytes(&record);
                writer.append(
                    RECORD_ORDER_CANCELLED, bytes,
                )?;
            }
            Event::OrderDone {
                handle: _,
                user_id,
                reason,
                filled_qty,
                remaining_qty,
                order_id_hi,
                order_id_lo,
            } => {
                let record = OrderDoneRecord {
                    preamble: make_prefix::<OrderDoneRecord>(),
                    ts_ns,
                    symbol_id,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    filled_qty: filled_qty.0,
                    remaining_qty: remaining_qty.0,
                    final_status: reason,
                    reduce_only: 0,
                    tif: 0,
                    post_only: 0,
                    _pad1: [0; 4],
                };
                let bytes = record_as_bytes(&record);
                writer.append(
                    RECORD_ORDER_DONE, bytes,
                )?;
            }
            Event::OrderFailed { .. } => {
                // OrderFailed is not persisted to WAL
            }
            Event::BBO { .. } => {
                // BBO is not emitted by ME, only by mktdata
            }
        }
    }
    Ok(())
}

/// Flush WAL if 10ms have elapsed since last flush.
#[inline]
pub fn flush_if_due(
    writer: &mut WalWriter,
    last_flush: &mut Instant,
) -> io::Result<()> {
    let now = Instant::now();
    if now.duration_since(*last_flush).as_millis() >= 10
    {
        writer.flush()?;
        *last_flush = now;
    }
    Ok(())
}

fn record_as_bytes<T>(record: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            record as *const T as *const u8,
            std::mem::size_of::<T>(),
        )
    }
}

fn make_prefix<T>() -> PayloadPreamble {
    PayloadPreamble {
        seq: 0,
        ver: 1,
        kind: 0,
        _pad0: 0,
        len: std::mem::size_of::<T>() as u32,
    }
}
