//! Write a handful of records to a fresh WAL, then read them
//! back via `WalReader`. Demonstrates the "wire = disk" claim:
//! the same `repr(C)` bytes you'd `sender.send(&mut rec)` are
//! the bytes sitting in the WAL file.
//!
//! Run:
//!   cargo run --example wal_replay -p rsx-cast

use rsx_cast::WalReader;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;

const STREAM_ID: u32 = 42;
const ROTATE_BYTES: u64 = 64 * 1024 * 1024;

fn fill(taker_oid: u64, price: i64, qty: i64) -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: taker_oid,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: Price(price),
        qty: Qty(qty),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    }
}

fn main() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    eprintln!("WAL dir: {}", dir.display());

    {
        let mut wal = WalWriter::new(
        STREAM_ID, dir, ROTATE_BYTES,
    )
        .unwrap();
        for i in 1..=5 {
            let mut rec = fill(1000 + i, 50_000 + i as i64, 100);
            {
                let framed = wal.prepare(&mut rec).unwrap();
                let seq = framed.seq;
                wal.append_framed(&framed).unwrap();
            }
            eprintln!("wrote seq={seq} taker_oid={}", rec.taker_order_id_lo);
        }
        wal.flush().unwrap();
        eprintln!("flushed");
    }

    eprintln!("---");

    let mut reader = WalReader::open_from_seq(STREAM_ID, 1, dir).unwrap();
    let mut n = 0;
    while let Some(raw) = reader.next().unwrap() {
        // Headers + payload bytes are identical to what would go over CMP.
        let seq = rsx_cast::wal::extract_seq(&raw.payload).unwrap_or(0);
        eprintln!(
            "read  seq={seq} type={} len={}",
            raw.header.record_type, raw.header.len
        );
        n += 1;
    }
    eprintln!("{n} records replayed");
    assert_eq!(n, 5, "expected 5 records, got {n}");
}
