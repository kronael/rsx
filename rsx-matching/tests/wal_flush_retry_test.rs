//! The ME stalls on a failed WAL flush and retries it (`flush_if_due`). A
//! flush that fails part-way through the write must still leave each record
//! in the segment once when the retry succeeds.
//!
//! The failure is a real one: `RLIMIT_FSIZE` makes `write` accept the bytes up
//! to the limit and then fail with `EFBIG`, until a second thread lifts it.
//! The limit is process-wide, so this binary holds exactly one test.

#![cfg(target_os = "linux")]

use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_matching::wal::flush_if_due;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

#[repr(C)]
struct RLimit {
    soft: u64,
    hard: u64,
}

const RLIMIT_FSIZE: i32 = 1;
const SIGXFSZ: i32 = 25;
const SIG_IGN: usize = 1;
const RLIM_INFINITY: u64 = u64::MAX;

extern "C" {
    fn setrlimit(resource: i32, limit: *const RLimit) -> i32;
    fn signal(signal: i32, handler: usize) -> usize;
}

fn set_file_size_limit(soft: u64) {
    let limit = RLimit {
        soft,
        hard: RLIM_INFINITY,
    };
    // SAFETY: `limit` is a valid `rlimit` for the call's duration.
    assert_eq!(unsafe { setrlimit(RLIMIT_FSIZE, &limit) }, 0);
}

fn fill() -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 1_000,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        taker_fee_bps: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: Price(50_000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        maker_fee_bps: 0,
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

#[test]
fn stalled_flush_retry_leaves_each_record_once() {
    const RECORDS: u64 = 20;
    const LIMIT_INSIDE_SEVENTH_FRAME: u64 = 1_000;
    const STALL: Duration = Duration::from_millis(30);

    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).unwrap();
    for _ in 0..RECORDS {
        let framed = writer.prepare(&mut fill()).unwrap();
        writer.append_framed(&framed).unwrap();
    }

    // SAFETY: `signal` with `SIG_IGN` has no preconditions.
    unsafe { signal(SIGXFSZ, SIG_IGN) };
    set_file_size_limit(LIMIT_INSIDE_SEVENTH_FRAME);
    let lifter = std::thread::spawn(|| {
        std::thread::sleep(STALL);
        set_file_size_limit(RLIM_INFINITY);
    });

    let mut last_flush = Instant::now();
    let now = last_flush + Duration::from_millis(10);
    let started = Instant::now();
    assert!(flush_if_due(&mut writer, &mut last_flush, now));
    lifter.join().unwrap();
    assert!(
        started.elapsed() >= STALL / 2,
        "the flush must have stalled"
    );

    let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut seqs = Vec::new();
    while let Some(raw) = reader.next().unwrap() {
        seqs.push(u64::from_le_bytes(raw.payload[0..8].try_into().unwrap()));
    }
    assert_eq!(seqs, (1..=RECORDS).collect::<Vec<_>>());
}
