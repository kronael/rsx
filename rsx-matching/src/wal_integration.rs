use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_types::NONE;
use rsx_book::snapshot;
use rsx_dxs::wal::WalWriter;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;
use tracing::warn;

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
            } => {
                let (reduce_only, tif) =
                    if maker_handle != NONE {
                        let slot =
                            book.orders.get(maker_handle);
                        (slot.is_reduce_only() as u8,
                         slot.tif)
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
                };
                writer.append(&mut record)?;
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
                let (reduce_only, tif) =
                    if handle != NONE {
                        let slot = book.orders.get(handle);
                        (slot.is_reduce_only() as u8,
                         slot.tif)
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
                writer.append(&mut record)?;
            }
            Event::OrderCancelled {
                handle,
                user_id,
                remaining_qty,
                reason,
                order_id_hi,
                order_id_lo,
            } => {
                let (reduce_only, tif, post_only) =
                    if handle != NONE {
                        let slot = book.orders.get(handle);
                        (slot.is_reduce_only() as u8,
                         slot.tif, 0u8)
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
                writer.append(&mut record)?;
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
                let (reduce_only, tif) =
                    if handle != NONE {
                        let slot = book.orders.get(handle);
                        (slot.is_reduce_only() as u8,
                         slot.tif)
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
                    final_status: reason,
                    reduce_only,
                    tif,
                    post_only: 0,
                    _pad1: [0; 4],
                };
                writer.append(&mut record)?;
            }
            Event::OrderFailed { .. } => {
                // OrderFailed is not persisted to WAL
            }
            Event::BBO { .. } => {
                // BBO not persisted to WAL (derived on replay)
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

/// Load book snapshot from
/// `{wal_dir}/{symbol_id}/snapshot.bin`.
/// Returns None if not found or corrupted.
pub fn load_snapshot(
    wal_dir: &str,
    symbol_id: u32,
) -> Option<Box<Orderbook>> {
    let path = PathBuf::from(wal_dir)
        .join(symbol_id.to_string())
        .join("snapshot.bin");
    let mut file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return None,
    };
    match snapshot::load(&mut file) {
        Ok(book) => {
            info!(
                "loaded snapshot from {}",
                path.display(),
            );
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

/// Save book snapshot to
/// `{wal_dir}/{symbol_id}/snapshot.bin`.
/// Uses atomic rename to avoid partial writes.
pub fn save_snapshot(
    book: &Orderbook,
    wal_dir: &str,
    symbol_id: u32,
) -> io::Result<()> {
    let dir = PathBuf::from(wal_dir)
        .join(symbol_id.to_string());
    let tmp = dir.join("snapshot.bin.tmp");
    let dest = dir.join("snapshot.bin");
    let mut file = fs::File::create(&tmp)?;
    snapshot::save(book, &mut file)?;
    file.sync_all()?;
    fs::rename(&tmp, &dest)?;
    Ok(())
}
