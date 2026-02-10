use rsx_dxs::records::*;
use rsx_dxs::wal::WalReader;
use rsx_dxs::wal::extract_seq;
use std::env;
use std::path::PathBuf;

fn record_name(rt: u16) -> &'static str {
    match rt {
        RECORD_FILL => "FILL",
        RECORD_BBO => "BBO",
        RECORD_ORDER_INSERTED => "ORDER_INSERTED",
        RECORD_ORDER_CANCELLED => "ORDER_CANCELLED",
        RECORD_ORDER_DONE => "ORDER_DONE",
        RECORD_CONFIG_APPLIED => "CONFIG_APPLIED",
        RECORD_CAUGHT_UP => "CAUGHT_UP",
        RECORD_ORDER_ACCEPTED => "ORDER_ACCEPTED",
        RECORD_MARK_PRICE => "MARK_PRICE",
        RECORD_ORDER_REQUEST => "ORDER_REQUEST",
        RECORD_ORDER_RESPONSE => "ORDER_RESPONSE",
        RECORD_CANCEL_REQUEST => "CANCEL_REQUEST",
        RECORD_ORDER_FAILED => "ORDER_FAILED",
        _ => "UNKNOWN",
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: wal_dump <stream_id> <wal_dir> \
             [from_seq]"
        );
        std::process::exit(1);
    }

    let stream_id: u32 =
        args[1].parse().expect("invalid stream_id");
    let wal_dir = PathBuf::from(&args[2]);
    let from_seq: u64 = if args.len() > 3 {
        args[3].parse().expect("invalid from_seq")
    } else {
        0
    };

    let mut reader = WalReader::open_from_seq(
        stream_id, from_seq, &wal_dir,
    )
    .expect("failed to open wal");

    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload)
            .unwrap_or(0);
        println!(
            "seq={:<8} type={:<18} len={:<4} \
             crc=0x{:08x}",
            seq,
            record_name(rt),
            len,
            raw.header.crc32,
        );
        count += 1;
    }
    eprintln!("total: {} records", count);
}
