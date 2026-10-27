#![allow(dead_code)] // each test target uses a subset of the helpers

#[path = "../../src/fixture_utils.rs"]
pub mod fixture_utils;
pub mod gateway_server;
pub mod md_server;

use rsx_cast::as_bytes;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::records::CaughtUpRecord;
use rsx_cast::records::RECORD_CAUGHT_UP;
use rsx_messages::BboRecord;
use rsx_messages::CancelRequest;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::MarkPriceRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_LIQUIDATION;
use rsx_messages::RECORD_MARK_PRICE;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_messages::RECORD_ORDER_RESPONSE;
use rsx_types::Price;
use rsx_types::Qty;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

pub const FIXTURE_STREAM: u32 = 1;
pub const FIXTURE_RECORDS: u64 = 15;
pub const UNKNOWN_RECORD_TYPE: u16 = 0xFF;

pub fn make_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rsx_cli_test_{}", name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .args(args)
        .output()
        .expect("failed to run rsx-cli")
}

pub fn write_frame(file: &mut File, record_type: u16, payload: &[u8]) {
    let crc = compute_crc32(payload);
    let header = WalHeader::new(record_type, payload.len() as u16, crc);
    file.write_all(header.to_bytes()).unwrap();
    file.write_all(payload).unwrap();
}

pub fn write_record<T: Copy>(file: &mut File, record_type: u16, record: &T) {
    write_frame(file, record_type, as_bytes(record));
}

fn zeroed<T: Copy>() -> T {
    // SAFETY: only used for repr(C) records made of integers and byte arrays,
    // for which the all-zero pattern is a valid value.
    unsafe { std::mem::zeroed() }
}

/// ORDER_REQUEST payload (64 bytes, repr(C, align(64))).
pub fn order_request_bytes(
    seq: u64,
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
    oid_hi: u64,
    oid_lo: u64,
    side: u8,
) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0..8].copy_from_slice(&seq.to_le_bytes());
    b[8..12].copy_from_slice(&user_id.to_le_bytes());
    b[12..16].copy_from_slice(&symbol_id.to_le_bytes());
    b[16..24].copy_from_slice(&price.to_le_bytes());
    b[24..32].copy_from_slice(&qty.to_le_bytes());
    b[32..40].copy_from_slice(&oid_hi.to_le_bytes());
    b[40..48].copy_from_slice(&oid_lo.to_le_bytes());
    b[48..56].copy_from_slice(&9999u64.to_le_bytes());
    b[56] = side;
    b
}

/// ORDER_RESPONSE payload (128 bytes, repr(C, align(64))).
pub fn order_response_bytes(
    seq: u64,
    ts_ns: u64,
    user_id: u32,
    symbol_id: u32,
    oid_hi: u64,
    oid_lo: u64,
    status: u8,
) -> [u8; 128] {
    let mut b = [0u8; 128];
    b[0..8].copy_from_slice(&seq.to_le_bytes());
    b[8..16].copy_from_slice(&ts_ns.to_le_bytes());
    b[16..20].copy_from_slice(&user_id.to_le_bytes());
    b[20..24].copy_from_slice(&symbol_id.to_le_bytes());
    b[24..32].copy_from_slice(&oid_hi.to_le_bytes());
    b[32..40].copy_from_slice(&oid_lo.to_le_bytes());
    b[40] = status;
    b
}

/// Writes `<wal_dir>/1/1_1_15.wal`: seq 1..=14 hold one record of each
/// decoded type, seq 15 an unknown type. `ts_ns` is `seq * 1000`. Records
/// start all-zero, so padding bytes and CRCs are the same on every run.
pub fn write_fixture_wal(wal_dir: &Path) -> PathBuf {
    let stream_dir = wal_dir.join(FIXTURE_STREAM.to_string());
    fs::create_dir_all(&stream_dir).unwrap();
    let path = stream_dir.join("1_1_15.wal");
    let mut file = File::create(&path).unwrap();

    let mut fill: FillRecord = zeroed();
    fill.seq = 1;
    fill.ts_ns = 1000;
    fill.symbol_id = 7;
    fill.taker_user_id = 100;
    fill.maker_user_id = 200;
    fill.taker_order_id_hi = 0xA1;
    fill.taker_order_id_lo = 0xA2;
    fill.maker_order_id_hi = 0xB1;
    fill.maker_order_id_lo = 0xB2;
    fill.price = Price(5000);
    fill.qty = Qty(10);
    fill.taker_side = 1;
    write_record(&mut file, RECORD_FILL, &fill);

    let mut bbo: BboRecord = zeroed();
    bbo.seq = 2;
    bbo.ts_ns = 2000;
    bbo.symbol_id = 7;
    bbo.bid_px = Price(4990);
    bbo.bid_qty = Qty(50);
    bbo.bid_count = 5;
    bbo.ask_px = Price(5010);
    bbo.ask_qty = Qty(30);
    bbo.ask_count = 3;
    write_record(&mut file, RECORD_BBO, &bbo);

    let mut inserted: OrderInsertedRecord = zeroed();
    inserted.seq = 3;
    inserted.ts_ns = 3000;
    inserted.symbol_id = 7;
    inserted.user_id = 100;
    inserted.order_id_hi = 0xC1;
    inserted.order_id_lo = 0xC2;
    inserted.price = Price(4990);
    inserted.qty = Qty(25);
    write_record(&mut file, RECORD_ORDER_INSERTED, &inserted);

    let mut cancelled: OrderCancelledRecord = zeroed();
    cancelled.seq = 4;
    cancelled.ts_ns = 4000;
    cancelled.symbol_id = 7;
    cancelled.user_id = 100;
    cancelled.order_id_hi = 0xC1;
    cancelled.order_id_lo = 0xC2;
    cancelled.remaining_qty = Qty(15);
    cancelled.reason = 2;
    write_record(&mut file, RECORD_ORDER_CANCELLED, &cancelled);

    let mut done: OrderDoneRecord = zeroed();
    done.seq = 5;
    done.ts_ns = 5000;
    done.symbol_id = 7;
    done.user_id = 200;
    done.order_id_hi = 0xB1;
    done.order_id_lo = 0xB2;
    done.filled_qty = Qty(10);
    done.final_status = 1;
    write_record(&mut file, RECORD_ORDER_DONE, &done);

    let mut config: ConfigAppliedRecord = zeroed();
    config.seq = 6;
    config.ts_ns = 6000;
    config.symbol_id = 8;
    config.config_version = 3;
    config.effective_at_ms = 1_700_000_000_000;
    config.applied_at_ns = 6001;
    write_record(&mut file, RECORD_CONFIG_APPLIED, &config);

    let mut caught_up: CaughtUpRecord = zeroed();
    caught_up.seq = 7;
    caught_up.ts_ns = 7000;
    caught_up.stream_id = FIXTURE_STREAM;
    caught_up.live_seq = 99;
    write_record(&mut file, RECORD_CAUGHT_UP, &caught_up);

    let mut accepted: OrderAcceptedRecord = zeroed();
    accepted.seq = 8;
    accepted.ts_ns = 8000;
    accepted.user_id = 100;
    accepted.symbol_id = 7;
    accepted.order_id_hi = 0xC1;
    accepted.order_id_lo = 0xC2;
    accepted.price = 4990;
    accepted.qty = 25;
    accepted.cid[..16].copy_from_slice(b"cid-fixture-0001");
    write_record(&mut file, RECORD_ORDER_ACCEPTED, &accepted);

    let mut mark: MarkPriceRecord = zeroed();
    mark.seq = 9;
    mark.ts_ns = 9000;
    mark.symbol_id = 8;
    mark.mark_price = Price(5005);
    mark.source_mask = 3;
    mark.source_count = 2;
    write_record(&mut file, RECORD_MARK_PRICE, &mark);

    let request = order_request_bytes(10, 300, 9, 6000, 40, 0xD1, 0xD2, 1);
    write_frame(&mut file, RECORD_ORDER_REQUEST, &request);

    let response = order_response_bytes(11, 11_000, 300, 9, 0xD1, 0xD2, 2);
    write_frame(&mut file, RECORD_ORDER_RESPONSE, &response);

    let mut cancel: CancelRequest = zeroed();
    cancel.seq = 12;
    cancel.ts_ns = 12_000;
    cancel.user_id = 100;
    cancel.symbol_id = 7;
    cancel.order_id_hi = 0xC1;
    cancel.order_id_lo = 0xC2;
    write_record(&mut file, RECORD_CANCEL_REQUEST, &cancel);

    let mut failed: OrderFailedRecord = zeroed();
    failed.seq = 13;
    failed.ts_ns = 13_000;
    failed.user_id = 100;
    failed.order_id_hi = 0xE1;
    failed.order_id_lo = 0xE2;
    failed.reason = 4;
    write_record(&mut file, RECORD_ORDER_FAILED, &failed);

    let mut liquidation: LiquidationRecord = zeroed();
    liquidation.seq = 14;
    liquidation.ts_ns = 14_000;
    liquidation.user_id = 500;
    liquidation.symbol_id = 9;
    liquidation.status = 1;
    liquidation.side = 1;
    liquidation.round = 2;
    liquidation.qty = 60;
    liquidation.price = 5900;
    liquidation.slip_bps = 12;
    write_record(&mut file, RECORD_LIQUIDATION, &liquidation);

    let mut unknown = [0u8; 16];
    unknown[..8].copy_from_slice(&15u64.to_le_bytes());
    unknown[8..16].copy_from_slice(&15_000u64.to_le_bytes());
    write_frame(&mut file, UNKNOWN_RECORD_TYPE, &unknown);

    file.sync_all().unwrap();
    path
}

/// Flips one payload byte of the `index`th record (0-based) of a segment, so
/// its CRC no longer matches.
pub fn corrupt_record(path: &Path, index: usize) {
    let mut bytes = fs::read(path).unwrap();
    let mut offset = 0;
    for _ in 0..index {
        let len = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]) as usize;
        offset += WalHeader::SIZE + len;
    }
    bytes[offset + WalHeader::SIZE + 1] ^= 0xFF;
    fs::write(path, bytes).unwrap();
}
