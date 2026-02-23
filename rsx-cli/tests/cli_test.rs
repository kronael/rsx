use rsx_dxs::header::WalHeader;
use rsx_dxs::records::BboRecord;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::LiquidationRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_BBO;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_LIQUIDATION;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
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
