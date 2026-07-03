//! Cold-start WAL replay: pre-write N=10k records, then
//! time `WalReader::next()` to EOF. This is the cost an ME
//! restart pays before serving its first order.
//!
//! The bench writes a mix of OrderAccepted + Fill + BBO
//! records that approximate the steady-state record-type
//! distribution. Each iter creates a fresh WalReader and
//! drains it.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::path::PathBuf;

#[path = "harness.rs"]
mod harness;

const SYMBOL_ID: u32 = 77;
const N: u64 = 10_000;

fn populate_wal(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();

    let mut writer = WalWriter::new(
        SYMBOL_ID, dir, 64 * 1024 * 1024,
    )
    .expect("wal writer");

    for i in 0..N {
        // 1 accepted + 1 fill + 1 bbo per "logical order".
        let mut accepted = OrderAcceptedRecord {
            seq: 0,
            ts_ns: 1_700_000_000_000 + i,
            user_id: (i % 1024) as u32 + 1,
            symbol_id: SYMBOL_ID,
            order_id_hi: 0,
            order_id_lo: i,
            price: 100_000 + (i % 100) as i64,
            qty: 10,
            side: (i % 2) as u8,
            tif: 0,
            reduce_only: 0,
            post_only: 0,
            cid: [0; 20],
        };
        {
            let framed = writer.prepare(&mut accepted).unwrap();
            writer.append_framed(&framed).unwrap();
        }

        let mut fill = FillRecord {
            seq: 0,
            ts_ns: 1_700_000_000_001 + i,
            symbol_id: SYMBOL_ID,
            taker_user_id: (i % 1024) as u32 + 1,
            maker_user_id: ((i + 1) % 1024) as u32 + 1,
            _pad0: 0,
            taker_order_id_hi: 0,
            taker_order_id_lo: i,
            maker_order_id_hi: 0,
            maker_order_id_lo: i + 1_000_000,
            price: Price(100_000 + (i % 100) as i64),
            qty: Qty(5),
            taker_side: (i % 2) as u8,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
            taker_ts_ns: 1_700_000_000_000 + i,
        };
        {
            let framed = writer.prepare(&mut fill).unwrap();
            writer.append_framed(&framed).unwrap();
        }

        let mut bbo = BboRecord {
            seq: 0,
            ts_ns: 1_700_000_000_002 + i,
            symbol_id: SYMBOL_ID,
            _pad0: 0,
            bid_px: Price(99_999),
            bid_qty: Qty(50),
            bid_count: 5,
            _pad1: 0,
            ask_px: Price(100_001),
            ask_qty: Qty(50),
            ask_count: 5,
            _pad2: 0,
        };
        {
            let framed = writer.prepare(&mut bbo).unwrap();
            writer.append_framed(&framed).unwrap();
        }
    }
    writer.flush().expect("flush");
}

fn bench_wal_replay(c: &mut Criterion) {
    harness::pin();
    let dir = PathBuf::from("./tmp/bench_wal_replay");
    populate_wal(&dir);

    c.bench_function("wal_replay_30k_records", |b| {
        b.iter(|| {
            let mut reader = WalReader::open_from_seq(
                SYMBOL_ID, 0, &dir,
            )
            .expect("open");
            let mut count = 0_u64;
            while let Ok(Some(rec)) = reader.next() {
                black_box(&rec.header);
                count += 1;
            }
            black_box(count);
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets = bench_wal_replay
}
criterion_main!(benches);
