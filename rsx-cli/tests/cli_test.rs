use rsx_dxs::encode_utils::compute_crc32;
use rsx_dxs::header::WalHeader;
use rsx_dxs::records::BboRecord;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::LiquidationRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_BBO;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_LIQUIDATION;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
use rsx_dxs::wal::extract_seq;
use rsx_dxs::wal::WalReader;
use rsx_types::Price;
use rsx_types::Qty;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn make_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("rsx_cli_test_{}", name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_record_bytes<T: Copy>(
    file: &mut File,
    rt: u16,
    record: &T,
) {
    let size = std::mem::size_of::<T>();
    let header = WalHeader::new(rt, size as u16, 0xAABBCCDD);
    file.write_all(&header.to_bytes()).unwrap();
    let ptr = record as *const T as *const u8;
    let bytes =
        unsafe { std::slice::from_raw_parts(ptr, size) };
    file.write_all(bytes).unwrap();
}

fn write_test_wal(path: &PathBuf, records: usize) {
    let mut file = File::create(path).unwrap();
    for i in 0..records {
        let header = WalHeader::new(RECORD_BBO, 8, i as u32);
        let payload = vec![i as u8; 8];
        file.write_all(&header.to_bytes()).unwrap();
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
    let header = WalHeader::new(
        RECORD_FILL,
        100,
        0xDEADBEEF,
    );

    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), 16);

    let record_type =
        u16::from_le_bytes([bytes[0], bytes[1]]);
    let len = u16::from_le_bytes([bytes[2], bytes[3]]);
    let crc32 = u32::from_le_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7],
    ]);

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
        file.write_all(&header.to_bytes()).unwrap();
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
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).unwrap();

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
        _pad0: 0,
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
        _pad1: [0; 4],
    };
    write_record_bytes(&mut file, RECORD_FILL, &fill);
    file.sync_all().unwrap();
    drop(file);

    // parse manually like dump_file does
    let buf = fs::read(&file_path).unwrap();
    let rt = u16::from_le_bytes([buf[0], buf[1]]);
    let len =
        u16::from_le_bytes([buf[2], buf[3]]) as usize;
    let payload = &buf[16..16 + len];

    assert_eq!(rt, RECORD_FILL);
    let decoded: FillRecord = unsafe {
        std::ptr::read(payload.as_ptr() as *const _)
    };
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
    };
    write_record_bytes(&mut file, RECORD_BBO, &bbo);
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let len =
        u16::from_le_bytes([buf[2], buf[3]]) as usize;
    let payload = &buf[16..16 + len];
    let decoded: BboRecord = unsafe {
        std::ptr::read(payload.as_ptr() as *const _)
    };
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
    write_record_bytes(
        &mut file,
        RECORD_ORDER_INSERTED,
        &rec,
    );
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let len =
        u16::from_le_bytes([buf[2], buf[3]]) as usize;
    let payload = &buf[16..16 + len];
    let decoded: OrderInsertedRecord = unsafe {
        std::ptr::read(payload.as_ptr() as *const _)
    };
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
    };
    write_record_bytes(
        &mut file,
        RECORD_LIQUIDATION,
        &rec,
    );
    file.sync_all().unwrap();
    drop(file);

    let buf = fs::read(&file_path).unwrap();
    let len =
        u16::from_le_bytes([buf[2], buf[3]]) as usize;
    let payload = &buf[16..16 + len];
    let decoded: LiquidationRecord = unsafe {
        std::ptr::read(payload.as_ptr() as *const _)
    };
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

/// Run `rsx-cli dump <file>` as a subprocess and return stdout.
fn run_dump(file: &std::path::Path) -> String {
    let bin = env!("CARGO_BIN_EXE_rsx-cli");
    let out = std::process::Command::new(bin)
        .arg("dump")
        .arg(file)
        .output()
        .expect("failed to run rsx-cli dump");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Write a 16-byte WAL header + `payload` to a file.
fn write_raw_record(
    file: &mut File,
    rt: u16,
    payload: &[u8],
) {
    use rsx_dxs::encode_utils::compute_crc32;
    use rsx_dxs::header::WalHeader;
    let crc = compute_crc32(payload);
    let header =
        WalHeader::new(rt, payload.len() as u16, crc);
    file.write_all(&header.to_bytes()).unwrap();
    file.write_all(payload).unwrap();
}

/// Build ORDER_REQUEST payload (64 bytes, repr(C,align(64))).
/// Layout from local OrderRequestRecord in main.rs:
///   seq(8) user_id(4) symbol_id(4) price(8) qty(8)
///   order_id_hi(8) order_id_lo(8) timestamp_ns(8)
///   side(1) tif(1) reduce_only(1) post_only(1)
///   is_liquidation(1) _pad(3)
fn order_request_bytes(
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

/// Build ORDER_RESPONSE payload (128 bytes, repr(C,align(64))).
/// Layout from local OrderResponseRecord in main.rs:
///   seq(8) ts_ns(8) user_id(4) symbol_id(4)
///   order_id_hi(8) order_id_lo(8) status(1) _pad(39)
/// + align(64) padding to 128 bytes total.
fn order_response_bytes(
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

/// `rsx-cli dump` prints ORDER_REQUEST records with key fields.
#[test]
fn test_dump_order_request_decodes() {
    let dir = make_test_dir("dump_order_request");
    let path = dir.join("req.wal");
    let mut file = File::create(&path).unwrap();
    let payload = order_request_bytes(
        5, 10, 2, 50000, 100, 0xAA, 0xBB, 1,
    );
    write_raw_record(
        &mut file,
        rsx_dxs::records::RECORD_ORDER_REQUEST,
        &payload,
    );
    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);
    // Must decode ORDER_REQUEST, not fall through to UNKNOWN.
    assert!(
        out.contains("ORDER_REQUEST"),
        "expected ORDER_REQUEST in output, got: {}", out
    );
    // sym and user fields must appear.
    assert!(
        out.contains("\"symbol_id\":2"),
        "expected symbol_id=2, got: {}", out
    );
    assert!(
        out.contains("\"user_id\":10"),
        "expected user_id=10, got: {}", out
    );
    assert!(
        out.contains("\"price\":50000"),
        "expected price=50000, got: {}", out
    );
}

/// `rsx-cli dump` prints ORDER_RESPONSE records with key fields.
#[test]
fn test_dump_order_response_decodes() {
    let dir = make_test_dir("dump_order_response");
    let path = dir.join("resp.wal");
    let mut file = File::create(&path).unwrap();
    let payload = order_response_bytes(
        6, 2000, 20, 3, 0xCC, 0xDD, 1,
    );
    write_raw_record(
        &mut file,
        rsx_dxs::records::RECORD_ORDER_RESPONSE,
        &payload,
    );
    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);
    assert!(
        out.contains("ORDER_RESPONSE"),
        "expected ORDER_RESPONSE in output, got: {}", out
    );
    assert!(
        out.contains("\"symbol_id\":3"),
        "expected symbol_id=3, got: {}", out
    );
    assert!(
        out.contains("\"user_id\":20"),
        "expected user_id=20, got: {}", out
    );
    assert!(
        out.contains("\"status\":1"),
        "expected status=1, got: {}", out
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
    write_raw_record(&mut file, 0xFF, &payload);
    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);
    assert!(
        out.contains("UNKNOWN"),
        "expected UNKNOWN in output for unrecognised rt, \
         got: {}", out
    );
}

/// Combined: one file with LIQUIDATION + ORDER_REQUEST +
/// ORDER_RESPONSE; all three must appear decoded in output.
#[test]
fn test_dump_three_new_types_combined() {
    use rsx_dxs::records::LiquidationRecord;
    use rsx_dxs::records::RECORD_LIQUIDATION;
    use rsx_dxs::records::RECORD_ORDER_REQUEST;
    use rsx_dxs::records::RECORD_ORDER_RESPONSE;

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
    };
    write_record_bytes(&mut file, RECORD_LIQUIDATION, &liq);

    // ORDER_REQUEST
    let req = order_request_bytes(
        2, 7, 1, 4000, 50, 0x11, 0x22, 0,
    );
    write_raw_record(&mut file, RECORD_ORDER_REQUEST, &req);

    // ORDER_RESPONSE
    let resp =
        order_response_bytes(3, 3000, 7, 1, 0x11, 0x22, 0);
    write_raw_record(&mut file, RECORD_ORDER_RESPONSE, &resp);

    file.sync_all().unwrap();
    drop(file);

    let out = run_dump(&path);

    assert!(
        out.contains("LIQUIDATION"),
        "expected LIQUIDATION, got: {}", out
    );
    assert!(
        out.contains("ORDER_REQUEST"),
        "expected ORDER_REQUEST, got: {}", out
    );
    assert!(
        out.contains("ORDER_RESPONSE"),
        "expected ORDER_RESPONSE, got: {}", out
    );
    // Ensure none are falling through to UNKNOWN.
    assert!(
        !out.contains("UNKNOWN"),
        "unexpected UNKNOWN in output: {}", out
    );
    // All three records must be present (3 JSON lines in stdout).
    let lines: Vec<&str> =
        out.lines().filter(|l| l.starts_with('{')).collect();
    assert_eq!(
        lines.len(),
        3,
        "expected 3 decoded records, got: {}", out
    );
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
    let write_mini =
        |f: &mut File, rt: u16, seq: u64| {
            let mut payload = [0u8; 16];
            payload[..8].copy_from_slice(
                &seq.to_le_bytes(),
            );
            let crc = compute_crc32(&payload);
            let header =
                WalHeader::new(rt, 16, crc);
            f.write_all(&header.to_bytes()).unwrap();
            f.write_all(&payload).unwrap();
        };

    write_mini(&mut file, RECORD_FILL, 1);
    write_mini(&mut file, RECORD_BBO, 2);
    write_mini(&mut file, RECORD_BBO, 3);
    write_mini(&mut file, RECORD_FILL, 4);
    file.sync_all().unwrap();
    drop(file);

    // Simulate the fixed follow loop.
    let mut reader =
        WalReader::open_from_seq(1, 0, &dir).unwrap();
    let mut next_seq: u64 = 0;
    let mut matched_seqs: Vec<u64> = Vec::new();

    loop {
        match reader.next() {
            Ok(Some(raw)) => {
                let rt = raw.header.record_type;
                let seq =
                    extract_seq(&raw.payload).unwrap_or(0);
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
    assert_eq!(
        next_seq, 5,
        "next_seq should advance past filtered records"
    );

    // Exactly the two FILLs, in order, no duplicates.
    assert_eq!(matched_seqs, vec![1, 4]);
}
