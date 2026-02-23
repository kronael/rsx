use clap::Parser;
use clap::Subcommand;
use rsx_dxs::records::*;
use rsx_dxs::wal::extract_seq;
use rsx_dxs::wal::WalReader;
use serde_json::json;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rsxcli", about = "RSX CLI tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Dump WAL records for a stream
    WalDump {
        /// Stream ID to read
        stream_id: u32,
        /// WAL directory path
        wal_dir: PathBuf,
        /// Start from this sequence number
        #[arg(default_value = "0")]
        from_seq: u64,
        /// Output as JSON lines (default: text)
        #[arg(long)]
        json: bool,
    },
    /// Dump a single WAL file as JSON lines
    Dump {
        /// WAL file path
        file: PathBuf,
    },
}

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
        RECORD_LIQUIDATION => "LIQUIDATION",
        _ => "UNKNOWN",
    }
}

fn oid_hex(hi: u64, lo: u64) -> String {
    format!("{:016x}{:016x}", hi, lo)
}

/// Decode payload bytes into record-specific fields.
/// Returns (text_suffix, json_fields) for the record type.
fn decode_payload(
    rt: u16,
    payload: &[u8],
) -> (String, Value) {
    unsafe {
        match rt {
            RECORD_FILL
                if payload.len()
                    >= std::mem::size_of::<FillRecord>() =>
            {
                let r: FillRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} taker={} maker={} px={} \
                     qty={} side={} oid={}",
                    r.symbol_id,
                    r.taker_user_id,
                    r.maker_user_id,
                    r.price.0,
                    r.qty.0,
                    r.taker_side,
                    oid_hex(
                        r.taker_order_id_hi,
                        r.taker_order_id_lo,
                    ),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "taker_user_id": r.taker_user_id,
                    "maker_user_id": r.maker_user_id,
                    "price": r.price.0,
                    "qty": r.qty.0,
                    "taker_side": r.taker_side,
                    "taker_oid": oid_hex(
                        r.taker_order_id_hi,
                        r.taker_order_id_lo,
                    ),
                    "maker_oid": oid_hex(
                        r.maker_order_id_hi,
                        r.maker_order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_BBO
                if payload.len()
                    >= std::mem::size_of::<BboRecord>() =>
            {
                let r: BboRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} bid={}x{} ask={}x{}",
                    r.symbol_id,
                    r.bid_px.0,
                    r.bid_qty.0,
                    r.ask_px.0,
                    r.ask_qty.0,
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "bid_px": r.bid_px.0,
                    "bid_qty": r.bid_qty.0,
                    "bid_count": r.bid_count,
                    "ask_px": r.ask_px.0,
                    "ask_qty": r.ask_qty.0,
                    "ask_count": r.ask_count,
                });
                (txt, j)
            }
            RECORD_ORDER_INSERTED
                if payload.len()
                    >= std::mem::size_of::<
                        OrderInsertedRecord,
                    >() =>
            {
                let r: OrderInsertedRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} px={} qty={} \
                     side={} oid={}",
                    r.symbol_id,
                    r.user_id,
                    r.price.0,
                    r.qty.0,
                    r.side,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "price": r.price.0,
                    "qty": r.qty.0,
                    "side": r.side,
                    "oid": oid_hex(
                        r.order_id_hi,
                        r.order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_ORDER_CANCELLED
                if payload.len()
                    >= std::mem::size_of::<
                        OrderCancelledRecord,
                    >() =>
            {
                let r: OrderCancelledRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} remain={} reason={} \
                     oid={}",
                    r.symbol_id,
                    r.user_id,
                    r.remaining_qty.0,
                    r.reason,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "remaining_qty": r.remaining_qty.0,
                    "reason": r.reason,
                    "oid": oid_hex(
                        r.order_id_hi,
                        r.order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_ORDER_DONE
                if payload.len()
                    >= std::mem::size_of::<
                        OrderDoneRecord,
                    >() =>
            {
                let r: OrderDoneRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} filled={} remain={} \
                     status={} oid={}",
                    r.symbol_id,
                    r.user_id,
                    r.filled_qty.0,
                    r.remaining_qty.0,
                    r.final_status,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "filled_qty": r.filled_qty.0,
                    "remaining_qty": r.remaining_qty.0,
                    "final_status": r.final_status,
                    "oid": oid_hex(
                        r.order_id_hi,
                        r.order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_CONFIG_APPLIED
                if payload.len()
                    >= std::mem::size_of::<
                        ConfigAppliedRecord,
                    >() =>
            {
                let r: ConfigAppliedRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} version={}",
                    r.symbol_id, r.config_version,
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "config_version": r.config_version,
                    "effective_at_ms": r.effective_at_ms,
                    "applied_at_ns": r.applied_at_ns,
                });
                (txt, j)
            }
            RECORD_CAUGHT_UP
                if payload.len()
                    >= std::mem::size_of::<
                        CaughtUpRecord,
                    >() =>
            {
                let r: CaughtUpRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " stream={} live_seq={}",
                    r.stream_id, r.live_seq,
                );
                let j = json!({
                    "stream_id": r.stream_id,
                    "live_seq": r.live_seq,
                });
                (txt, j)
            }
            RECORD_ORDER_ACCEPTED
                if payload.len()
                    >= std::mem::size_of::<
                        OrderAcceptedRecord,
                    >() =>
            {
                let r: OrderAcceptedRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} px={} qty={} \
                     side={} oid={}",
                    r.symbol_id,
                    r.user_id,
                    r.price,
                    r.qty,
                    r.side,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "price": r.price,
                    "qty": r.qty,
                    "side": r.side,
                    "oid": oid_hex(
                        r.order_id_hi,
                        r.order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_MARK_PRICE
                if payload.len()
                    >= std::mem::size_of::<
                        MarkPriceRecord,
                    >() =>
            {
                let r: MarkPriceRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} mark={}",
                    r.symbol_id, r.mark_price.0,
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "mark_price": r.mark_price.0,
                    "source_mask": r.source_mask,
                    "source_count": r.source_count,
                });
                (txt, j)
            }
            RECORD_CANCEL_REQUEST
                if payload.len()
                    >= std::mem::size_of::<
                        CancelRequest,
                    >() =>
            {
                let r: CancelRequest =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} oid={}",
                    r.symbol_id,
                    r.user_id,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "oid": oid_hex(
                        r.order_id_hi,
                        r.order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_ORDER_FAILED
                if payload.len()
                    >= std::mem::size_of::<
                        OrderFailedRecord,
                    >() =>
            {
                let r: OrderFailedRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " user={} reason={} oid={}",
                    r.user_id,
                    r.reason,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "user_id": r.user_id,
                    "reason": r.reason,
                    "oid": oid_hex(
                        r.order_id_hi,
                        r.order_id_lo,
                    ),
                });
                (txt, j)
            }
            RECORD_LIQUIDATION
                if payload.len()
                    >= std::mem::size_of::<
                        LiquidationRecord,
                    >() =>
            {
                let r: LiquidationRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} status={} side={} \
                     round={} qty={} px={} slip={}",
                    r.symbol_id,
                    r.user_id,
                    r.status,
                    r.side,
                    r.round,
                    r.qty,
                    r.price,
                    r.slip_bps,
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "status": r.status,
                    "side": r.side,
                    "round": r.round,
                    "qty": r.qty,
                    "price": r.price,
                    "slip_bps": r.slip_bps,
                });
                (txt, j)
            }
            _ => (String::new(), json!({})),
        }
    }
}

fn wal_dump(
    stream_id: u32,
    wal_dir: PathBuf,
    from_seq: u64,
    json: bool,
) {
    let mut reader = WalReader::open_from_seq(
        stream_id, from_seq, &wal_dir,
    )
    .expect("failed to open wal");

    if json {
        dump_json(&mut reader);
    } else {
        dump_text(&mut reader);
    }
}

fn dump_text(reader: &mut WalReader) {
    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        let (fields, _) = decode_payload(rt, &raw.payload);

        println!(
            "seq={:<8} type={:<18} len={:<4} \
             crc=0x{:08x}{}",
            seq,
            record_name(rt),
            len,
            raw.header.crc32,
            fields,
        );
        count += 1;
    }
    eprintln!("total: {} records", count);
}

fn dump_json(reader: &mut WalReader) {
    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        let (_, fields) = decode_payload(rt, &raw.payload);

        let mut obj = json!({
            "seq": seq,
            "type": record_name(rt),
            "len": len,
            "crc32": format!("0x{:08x}", raw.header.crc32),
        });
        if let Value::Object(m) = fields {
            if let Value::Object(ref mut base) = obj {
                base.extend(m);
            }
        }
        println!("{}", obj);
        count += 1;
    }
    eprintln!("total: {} records", count);
}

fn dump_file(file: PathBuf) {
    use std::fs::File;
    use std::io::Read;

    let mut f = File::open(&file).expect("failed to open file");
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).expect("failed to read file");

    let mut offset = 0;
    let mut count = 0;

    while offset + 16 <= buf.len() {
        let header = &buf[offset..offset + 16];
        let rt = u16::from_le_bytes(
            [header[0], header[1]],
        );
        let len = u16::from_le_bytes(
            [header[2], header[3]],
        ) as usize;
        let crc32 = u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]);

        if len > 1_000_000 {
            eprintln!(
                "corrupt: record len {} at offset {}",
                len, offset,
            );
            break;
        }
        if offset + 16 + len > buf.len() {
            break;
        }

        let payload = &buf[offset + 16..offset + 16 + len];
        let seq = extract_seq(payload).unwrap_or(0);
        let (_, fields) = decode_payload(rt, payload);

        let mut obj = json!({
            "seq": seq,
            "type": record_name(rt),
            "len": len,
            "crc32": format!("0x{:08x}", crc32),
        });
        if let Value::Object(m) = fields {
            if let Value::Object(ref mut base) = obj {
                base.extend(m);
            }
        }
        println!("{}", obj);

        offset += 16 + len;
        count += 1;
    }
    eprintln!("total: {} records", count);
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::WalDump {
            stream_id,
            wal_dir,
            from_seq,
            json,
        } => wal_dump(stream_id, wal_dir, from_seq, json),
        Commands::Dump { file } => dump_file(file),
    }
}
