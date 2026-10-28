mod common;

use common::make_test_dir;
use common::order_request_bytes;
use common::order_response_bytes;
use common::run_cli;
use common::write_frame;
use common::write_record;
use rsx_cast::decode_payload;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::WalReader;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_LIQUIDATION;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::Price;
use rsx_types::Qty;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn write_test_wal(path: &PathBuf, records: usize) {
    let mut file = File::create(path).unwrap();
    for i in 0..records {
        let header = WalHeader::new(RECORD_BBO, 8, i as u32);
        let payload = vec![i as u8; 8];
        file.write_all(header.to_bytes()).unwrap();
        file.write_all(&payload).unwrap();
    }
    file.sync_all().unwrap();
}

#[test]
fn test_dump_file_parsing() {
    let dir = make_test_dir("dump_file_parsing");
    let file_path = dir.join("test.wal");
    write_test_wal(&file_path, 5);

    assert!(file_path.exists());
    let metadata = fs::metadata(&file_path).unwrap();
    assert_eq!(metadata.len(), 5 * (16 + 8));
}

#[test]
fn test_wal_header_format() {
    let header = WalHeader::new(RECORD_FILL, 100, 0xDEADBEEF);

    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), 16);

    let version = bytes[0];
    let record_type = u16::from_le_bytes([bytes[2], bytes[3]]);
    let len = u16::from_le_bytes([bytes[4], bytes[5]]);
    let crc32 = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);

    assert_eq!(version, 1, "WalVersion::V1");
    assert_eq!(record_type, RECORD_FILL);
    assert_eq!(len, 100);
    assert_eq!(crc32, 0xDEADBEEF);
}

#[test]
fn test_multiple_records_in_file() {
    let dir = make_test_dir("multiple_records");
    let file_path = dir.join("multi.wal");
    let mut file = File::create(&file_path).unwrap();

    for i in 0..3 {
        let header = WalHeader::new(RECORD_BBO, 4, i);
        let payload = vec![i as u8; 4];
        file.write_all(header.to_bytes()).unwrap();
        file.write_all(&payload).unwrap();
    }
    file.sync_all().unwrap();
    drop(file);

    let contents = fs::read(&file_path).unwrap();
    assert_eq!(contents.len(), 3 * (16 + 4));
}

#[test]
fn test_json_output_format() {
    let json_str = concat!(
        r#"{"seq":123,"type":"BBO","len":32,"#,
        r#""crc32":"0x12345678"}"#,
    );
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();

    assert_eq!(parsed["seq"], 123);
    assert_eq!(parsed["type"], "BBO");
    assert_eq!(parsed["len"], 32);
    assert_eq!(parsed["crc32"], "0x12345678");
}

#[test]
fn test_file_empty_handling() {
    let dir = make_test_dir("empty_handling");
    let empty_file = dir.join("empty.wal");
    File::create(&empty_file).unwrap();

    let contents = fs::read(&empty_file).unwrap();
    assert_eq!(contents.len(), 0);
}

#[test]
fn test_dump_file_decodes_fill_fields() {
    let dir = make_test_dir("decode_fill");
    let file_path = dir.join("fill.wal");
    let mut file = File::create(&file_path).unwrap();

    let fill = FillRecord {
        seq: 42,
        ts_ns: 1000,
        symbol_id: 7,
        taker_user_id: 100,
        maker_user_id: 200,
        taker_fee_bps: 0,
        taker_order_id_hi: 0xAA,
        taker_order_id_lo: 0xBB,
        maker_order_id_hi: 0xCC,
        maker_order_id_lo: 0xDD,
        price: Price(5000),
        qty: Qty(10),
        taker_side: 1,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        maker_fee_bps: 0,
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    };
    write_record(&mut file, RECORD_FILL, &fill);
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let hdr = WalHeader::from_bytes(&buf).unwrap();
    let payload = &buf[WalHeader::SIZE..WalHeader::SIZE + hdr.len as usize];

    assert_eq!(hdr.record_type, RECORD_FILL);
    let decoded = decode_payload::<FillRecord>(payload).unwrap();
    assert_eq!(decoded.seq, 42);
    assert_eq!(decoded.symbol_id, 7);
    assert_eq!(decoded.taker_user_id, 100);
    assert_eq!(decoded.maker_user_id, 200);
    assert_eq!(decoded.price.0, 5000);
    assert_eq!(decoded.qty.0, 10);
    assert_eq!(decoded.taker_side, 1);
}

#[test]
fn test_dump_file_decodes_bbo_fields() {
    let dir = make_test_dir("decode_bbo");
    let file_path = dir.join("bbo.wal");
    let mut file = File::create(&file_path).unwrap();

    let bbo = BboRecord {
        seq: 99,
        ts_ns: 2000,
        symbol_id: 3,
        _pad0: 0,
        bid_px: Price(4900),
        bid_qty: Qty(50),
        bid_count: 5,
        _pad1: 0,
        ask_px: Price(5100),
        ask_qty: Qty(30),
        ask_count: 3,
        _pad2: 0,
        _pad3: [0; 56],
    };
    write_record(&mut file, RECORD_BBO, &bbo);
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let hdr = WalHeader::from_bytes(&buf).unwrap();
    let payload = &buf[WalHeader::SIZE..WalHeader::SIZE + hdr.len as usize];
    let decoded = decode_payload::<BboRecord>(payload).unwrap();
    assert_eq!(decoded.seq, 99);
    assert_eq!(decoded.symbol_id, 3);
    assert_eq!(decoded.bid_px.0, 4900);
    assert_eq!(decoded.ask_px.0, 5100);
    assert_eq!(decoded.bid_qty.0, 50);
    assert_eq!(decoded.ask_qty.0, 30);
}

#[test]
fn test_dump_file_decodes_order_inserted_fields() {
    let dir = make_test_dir("decode_order_inserted");
    let file_path = dir.join("ins.wal");
    let mut file = File::create(&file_path).unwrap();

    let rec = OrderInsertedRecord {
        seq: 55,
        ts_ns: 3000,
        symbol_id: 2,
        user_id: 42,
        order_id_hi: 0x1111,
        order_id_lo: 0x2222,
        price: Price(6000),
        qty: Qty(25),
        side: 0,
        reduce_only: 1,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    write_record(&mut file, RECORD_ORDER_INSERTED, &rec);
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let hdr = WalHeader::from_bytes(&buf).unwrap();
    let payload = &buf[WalHeader::SIZE..WalHeader::SIZE + hdr.len as usize];
    let decoded = decode_payload::<OrderInsertedRecord>(payload).unwrap();
    assert_eq!(decoded.user_id, 42);
    assert_eq!(decoded.price.0, 6000);
    assert_eq!(decoded.qty.0, 25);
    assert_eq!(decoded.side, 0);
}

#[test]
fn test_dump_file_decodes_liquidation_fields() {
    let dir = make_test_dir("decode_liquidation");
    let file_path = dir.join("liq.wal");
    let mut file = File::create(&file_path).unwrap();

    let rec = LiquidationRecord {
        seq: 77,
        ts_ns: 4000,
        user_id: 300,
        symbol_id: 5,
        status: 1,
        side: 0,
        _pad0: [0; 2],
        round: 3,
        qty: 100,
        price: 5500,
        slip_bps: 25,
        _pad1: [0; 8],
    };
    write_record(&mut file, RECORD_LIQUIDATION, &rec);
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let hdr = WalHeader::from_bytes(&buf).unwrap();
    let payload = &buf[WalHeader::SIZE..WalHeader::SIZE + hdr.len as usize];
    let decoded = decode_payload::<LiquidationRecord>(payload).unwrap();
    assert_eq!(decoded.seq, 77);
    assert_eq!(decoded.user_id, 300);
    assert_eq!(decoded.symbol_id, 5);
    assert_eq!(decoded.status, 1);
    assert_eq!(decoded.round, 3);
    assert_eq!(decoded.qty, 100);
    assert_eq!(decoded.price, 5500);
    assert_eq!(decoded.slip_bps, 25);
}

#[test]
fn test_dump_file_json_includes_decoded_fields() {
    // Verify JSON output merges decoded fields
    use serde_json::json;
    use serde_json::Value;

    let mut base = json!({
        "seq": 42,
        "type": "FILL",
        "len": 64,
        "crc32": "0x00000000",
    });
    let fields = json!({
        "symbol_id": 7,
        "price": 5000,
    });

    if let Value::Object(m) = fields {
        if let Value::Object(ref mut b) = base {
            b.extend(m);
        }
    }

    assert_eq!(base["seq"], 42);
    assert_eq!(base["symbol_id"], 7);
    assert_eq!(base["price"], 5000);
    assert_eq!(base["type"], "FILL");
}

/// Run `rsx-cli wal file <file>` as a subprocess and return stdout.
fn run_dump(file: &std::path::Path) -> String {
    let out = run_cli(&["wal", "file", file.to_str().unwrap()]);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `rsx-cli wal file` prints ORDER_REQUEST records with key fields.
#[test]
fn test_dump_order_request_decodes() {
    let dir = make_test_dir("dump_order_request");
    let path = dir.join("req.wal");
    let mut file = File::create(&path).unwrap();
    let payload = order_request_bytes(5, 10, 2, 50000, 100, 0xAA, 0xBB, 1);
    write_frame(&mut file, rsx_messages::RECORD_ORDER_REQUEST, &payload);
    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);
    // Must decode ORDER_REQUEST, not fall through to UNKNOWN.
    assert!(
        out.contains("ORDER_REQUEST"),
        "expected ORDER_REQUEST in output, got: {}",
        out
    );
    // sym and user fields must appear.
    assert!(
        out.contains("\"symbol_id\":2"),
        "expected symbol_id=2, got: {}",
        out
    );
    assert!(
        out.contains("\"user_id\":10"),
        "expected user_id=10, got: {}",
        out
    );
    assert!(
        out.contains("\"price\":50000"),
        "expected price=50000, got: {}",
        out
    );
}

/// `rsx-cli wal file` prints ORDER_RESPONSE records with key fields.
#[test]
fn test_dump_order_response_decodes() {
    let dir = make_test_dir("dump_order_response");
    let path = dir.join("resp.wal");
    let mut file = File::create(&path).unwrap();
    let payload = order_response_bytes(6, 2000, 20, 3, 0xCC, 0xDD, 1);
    write_frame(&mut file, rsx_messages::RECORD_ORDER_RESPONSE, &payload);
    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);
    assert!(
        out.contains("ORDER_RESPONSE"),
        "expected ORDER_RESPONSE in output, got: {}",
        out
    );
    assert!(
        out.contains("\"symbol_id\":3"),
        "expected symbol_id=3, got: {}",
        out
    );
    assert!(
        out.contains("\"user_id\":20"),
        "expected user_id=20, got: {}",
        out
    );
    assert!(
        out.contains("\"status\":1"),
        "expected status=1, got: {}",
        out
    );
}

/// Unknown record types are printed as "UNKNOWN",
/// not silently dropped.
#[test]
fn test_dump_unknown_type_not_skipped() {
    let dir = make_test_dir("dump_unknown_type");
    let path = dir.join("unk.wal");
    let mut file = File::create(&path).unwrap();
    // rt=0xFF is not a known record type.
    let payload = [0u8; 16];
    write_frame(&mut file, 0xFF, &payload);
    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);
    assert!(
        out.contains("UNKNOWN"),
        "expected UNKNOWN in output for unrecognised rt, \
         got: {}",
        out
    );
}

/// Combined: one file with LIQUIDATION + ORDER_REQUEST +
/// ORDER_RESPONSE; all three must appear decoded in output.
#[test]
fn test_dump_three_new_types_combined() {
    use rsx_messages::LiquidationRecord;
    use rsx_messages::RECORD_LIQUIDATION;
    use rsx_messages::RECORD_ORDER_REQUEST;
    use rsx_messages::RECORD_ORDER_RESPONSE;

    let dir = make_test_dir("dump_three_types");
    let path = dir.join("combined.wal");
    let mut file = File::create(&path).unwrap();

    // LIQUIDATION
    let liq = LiquidationRecord {
        seq: 1,
        ts_ns: 1000,
        user_id: 5,
        symbol_id: 1,
        status: 2,
        side: 1,
        _pad0: [0; 2],
        round: 0,
        qty: 50,
        price: 4000,
        slip_bps: 10,
        _pad1: [0; 8],
    };
    write_record(&mut file, RECORD_LIQUIDATION, &liq);

    // ORDER_REQUEST
    let req = order_request_bytes(2, 7, 1, 4000, 50, 0x11, 0x22, 0);
    write_frame(&mut file, RECORD_ORDER_REQUEST, &req);

    // ORDER_RESPONSE
    let resp = order_response_bytes(3, 3000, 7, 1, 0x11, 0x22, 0);
    write_frame(&mut file, RECORD_ORDER_RESPONSE, &resp);

    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);

    assert!(
        out.contains("LIQUIDATION"),
        "expected LIQUIDATION, got: {}",
        out
    );
    assert!(
        out.contains("ORDER_REQUEST"),
        "expected ORDER_REQUEST, got: {}",
        out
    );
    assert!(
        out.contains("ORDER_RESPONSE"),
        "expected ORDER_RESPONSE, got: {}",
        out
    );
    // Ensure none are falling through to UNKNOWN.
    assert!(
        !out.contains("UNKNOWN"),
        "unexpected UNKNOWN in output: {}",
        out
    );
    // All three records must be present (3 JSON lines in stdout).
    let lines: Vec<&str> = out.lines().filter(|line| line.starts_with('{')).collect();
    assert_eq!(lines.len(), 3, "expected 3 decoded records, got: {}", out);
}

/// Regression: in --follow mode next_seq must advance even when
/// a record is excluded by a filter. If it does not, the reader
/// will re-open from the last matched seq on EOF, replaying every
/// filtered-out record between that point and the current tip.
///
/// Scenario: WAL contains FILL(1) BBO(2) BBO(3) FILL(4).
/// Filter: FILL only.
/// Expected: next_seq = 5 after consuming all four records,
///           matched_seqs = [1, 4] (no duplicates).
#[test]
fn test_follow_next_seq_advances_past_filtered_records() {
    let dir = make_test_dir("follow_next_seq");

    // Write one WAL segment for stream 1.
    // WalReader expects files named "<stream_id>-<seq>.wal" or
    // similar; use the simplest approach: write a raw file and
    // open it via WalReader::open_from_seq.
    // WalReader looks in <wal_dir>/<stream_id>/ for segments
    // named "<stream_id>_<first_seq>_<last_seq>.wal".
    let hot_dir = dir.join("1");
    fs::create_dir_all(&hot_dir).unwrap();
    let seg = hot_dir.join("1_1_4.wal");
    let mut file = File::create(&seg).unwrap();

    // Helper: write a minimal record with a given seq.
    // 16-byte payload: seq(8) + ts_ns(8). CRC computed
    // from payload so WalReader's CRC check passes.
    let write_mini = |f: &mut File, rt: u16, seq: u64| {
        let mut payload = [0u8; 16];
        payload[..8].copy_from_slice(&seq.to_le_bytes());
        let crc = compute_crc32(&payload);
        let header = WalHeader::new(rt, 16, crc);
        f.write_all(header.to_bytes()).unwrap();
        f.write_all(&payload).unwrap();
    };

    write_mini(&mut file, RECORD_FILL, 1);
    write_mini(&mut file, RECORD_BBO, 2);
    write_mini(&mut file, RECORD_BBO, 3);
    write_mini(&mut file, RECORD_FILL, 4);
    file.sync_all().unwrap();
    drop(file);

    // Exercise the follow loop.
    let mut reader = WalReader::open_from_seq(1, 0, &dir).unwrap();
    let mut next_seq: u64 = 0;
    let mut matched_seqs: Vec<u64> = Vec::new();

    loop {
        match reader.next() {
            Ok(Some(raw)) => {
                let rt = raw.header.record_type;
                let seq = extract_seq(&raw.payload).unwrap_or(0);
                // Fixed: always advance before filter check.
                next_seq = seq + 1;
                if rt == RECORD_FILL {
                    matched_seqs.push(seq);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    // next_seq must be 5 (past the last record seq=4),
    // not 2 (which the buggy code would leave it at after
    // matching seq=1 and then hitting filtered-out seq=2,3).
    assert_eq!(next_seq, 5, "next_seq should advance past filtered records");

    // Exactly the two FILLs, in order, no duplicates.
    assert_eq!(matched_seqs, vec![1, 4]);
}

fn write_mid_file_corruption(name: &str) -> PathBuf {
    let dir = make_test_dir(name);
    let hot_dir = dir.join("1");
    fs::create_dir_all(&hot_dir).unwrap();
    let mut file = File::create(hot_dir.join("1_1_3.wal")).unwrap();
    for seq in 1..=3u64 {
        let mut payload = [0u8; 16];
        payload[..8].copy_from_slice(&seq.to_le_bytes());
        let crc = compute_crc32(&payload);
        let header = WalHeader::new(RECORD_FILL, 16, crc);
        if seq == 2 {
            payload[8] ^= 0xFF;
        }
        file.write_all(header.to_bytes()).unwrap();
        file.write_all(&payload).unwrap();
    }
    dir
}

/// `rsx-cli wal dump` over a segment with a corrupt record followed by an
/// intact one prints the records before it, then exits 1 with the read error
/// instead of ending as if the WAL stopped there.
#[test]
fn test_wal_dump_fails_on_mid_file_corruption() {
    let dir = write_mid_file_corruption("wal_dump_mid_file_corruption");
    let dir = dir.to_str().unwrap();
    for args in [
        ["wal", "dump", "1", dir].as_slice(),
        ["wal", "dump", "1", dir, "--json"].as_slice(),
        ["wal", "stats", "1", dir].as_slice(),
    ] {
        let out = run_cli(args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(stderr.contains("crc mismatch"), "{args:?}: {stderr}");
    }
}

/// Under `--json` the failure is one `{"error": ...}` line on stderr and
/// stdout carries only the records printed before it.
#[test]
fn test_wal_dump_json_error_is_one_json_line() {
    let dir = write_mid_file_corruption("wal_dump_json_error");
    let out = run_cli(&["wal", "dump", "1", dir.to_str().unwrap(), "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert_eq!(stderr.trim().lines().count(), 1, "{stderr}");
    let error: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert!(error["error"].as_str().unwrap().contains("crc mismatch"));
}

#[test]
fn test_version_flag_prints_the_crate_version() {
    let out = run_cli(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("rsx-cli {}\n", env!("CARGO_PKG_VERSION"))
    );
}
