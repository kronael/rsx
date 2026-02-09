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
                maker_handle,
                taker_user_id,
                price,
                qty,
                side,
            } => {
                let record = FillRecord {
                    seq: 0, // assigned by WAL
                    ts_ns,
                    symbol_id,
                    maker_oid: maker_handle as u128,
                    taker_oid: taker_user_id as u128,
                    px: price.0,
                    qty: qty.0,
                    maker_side: side,
                    _pad1: [0; 7],
                };
                let bytes = record_as_bytes(&record);
                writer.append(RECORD_FILL, bytes)?;
            }
            Event::OrderInserted {
                handle,
                user_id,
                side,
                price,
                qty,
            } => {
                let record = OrderInsertedRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    oid: handle as u128,
                    user_id,
                    px: price.0,
                    qty: qty.0,
                    side,
                    _pad1: [0; 7],
                };
                let bytes = record_as_bytes(&record);
                writer.append(
                    RECORD_ORDER_INSERTED, bytes,
                )?;
            }
            Event::OrderCancelled {
                handle,
                user_id,
                remaining_qty,
            } => {
                let record = OrderCancelledRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    oid: handle as u128,
                    reason: 1, // cancelled
                    _pad1: [0; 7],
                };
                let _ = remaining_qty;
                let _ = user_id;
                let bytes = record_as_bytes(&record);
                writer.append(
                    RECORD_ORDER_CANCELLED, bytes,
                )?;
            }
            Event::OrderDone {
                handle,
                user_id,
                reason,
            } => {
                let record = OrderDoneRecord {
                    seq: 0,
                    ts_ns,
                    symbol_id,
                    oid: handle as u128,
                    remaining_qty: 0,
                    reason,
                    _pad1: [0; 7],
                };
                let _ = user_id;
                let bytes = record_as_bytes(&record);
                writer.append(
                    RECORD_ORDER_DONE, bytes,
                )?;
            }
            Event::OrderFailed { .. } => {
                // OrderFailed is not persisted to WAL
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
