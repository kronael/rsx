use clap::Parser;
use clap::Subcommand;
use rsx_dxs::records::*;
use rsx_dxs::wal::extract_seq;
use rsx_dxs::wal::WalReader;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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
        /// Only emit records of this type (repeatable, OR logic)
        #[arg(long = "type", value_name = "TYPE")]
        record_types: Vec<String>,
        /// Filter by symbol_id
        #[arg(long)]
        symbol: Option<u32>,
        /// Filter by user_id
        #[arg(long)]
        user: Option<u32>,
        /// Skip records with ts_ns < value
        #[arg(long)]
        from_ts: Option<u64>,
        /// Stop after ts_ns > value
        #[arg(long)]
        to_ts: Option<u64>,
        /// Print per-type counts instead of records
        #[arg(long, conflicts_with = "follow")]
        stats: bool,
        /// Follow WAL for new records (not yet implemented)
        #[arg(long, conflicts_with = "stats")]
        follow: bool,
        /// Divide raw i64 prices by this for text display
        #[arg(long)]
        tick_size: Option<f64>,
        /// Divide raw i64 quantities by this for text display
        #[arg(long)]
        lot_size: Option<f64>,
    },
    /// Dump a single WAL file as JSON lines
    Dump {
        /// WAL file path
        file: PathBuf,
    },
}

struct Filters {
    record_types: Vec<u16>,
    symbol: Option<u32>,
    user: Option<u32>,
    from_ts: Option<u64>,
    to_ts: Option<u64>,
}

impl Filters {
    fn from_args(
        record_types: Vec<String>,
        symbol: Option<u32>,
        user: Option<u32>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Self {
        let rts = record_types
            .iter()
            .filter_map(|s| name_to_record_type(s.as_str()))
            .collect();
        Filters { record_types: rts, symbol, user, from_ts, to_ts }
    }

    fn matches(&self, rt: u16, payload: &[u8]) -> bool {
        if !self.record_types.is_empty()
            && !self.record_types.contains(&rt)
        {
            return false;
        }
        let ts = extract_ts_ns(payload);
        if let (Some(from), Some(ts)) = (self.from_ts, ts) {
            if ts < from {
                return false;
            }
        }
        if let (Some(to), Some(ts)) = (self.to_ts, ts) {
            if ts > to {
                return false;
            }
        }
        if let Some(sym) = self.symbol {
            match extract_symbol_id(rt, payload) {
                Some(s) if s == sym => {}
                Some(_) => return false,
                None => return false,
            }
        }
        if let Some(uid) = self.user {
            match extract_user_id(rt, payload) {
                Some(u) if u == uid => {}
                Some(_) => return false,
                None => return false,
            }
        }
        true
    }
}

struct DisplayScale {
    tick: f64,
    lot: f64,
}

impl DisplayScale {
    fn new(tick_size: Option<f64>, lot_size: Option<f64>) -> Self {
        DisplayScale {
            tick: tick_size.unwrap_or(1.0),
            lot: lot_size.unwrap_or(1.0),
        }
    }

    fn px(&self, raw: i64) -> f64 {
        raw as f64 / self.tick
    }

    fn qty(&self, raw: i64) -> f64 {
        raw as f64 / self.lot
    }
}

fn name_to_record_type(name: &str) -> Option<u16> {
    match name {
        "FILL" => Some(RECORD_FILL),
        "BBO" => Some(RECORD_BBO),
        "ORDER_INSERTED" => Some(RECORD_ORDER_INSERTED),
        "ORDER_CANCELLED" => Some(RECORD_ORDER_CANCELLED),
        "ORDER_DONE" => Some(RECORD_ORDER_DONE),
        "CONFIG_APPLIED" => Some(RECORD_CONFIG_APPLIED),
        "CAUGHT_UP" => Some(RECORD_CAUGHT_UP),
        "ORDER_ACCEPTED" => Some(RECORD_ORDER_ACCEPTED),
        "MARK_PRICE" => Some(RECORD_MARK_PRICE),
        "ORDER_REQUEST" => Some(RECORD_ORDER_REQUEST),
        "ORDER_RESPONSE" => Some(RECORD_ORDER_RESPONSE),
        "CANCEL_REQUEST" => Some(RECORD_CANCEL_REQUEST),
        "ORDER_FAILED" => Some(RECORD_ORDER_FAILED),
        "LIQUIDATION" => Some(RECORD_LIQUIDATION),
        _ => {
            eprintln!("unknown record type: {}", name);
            None
        }
    }
}

/// Extract ts_ns from bytes [8..16] (present in all records).
fn extract_ts_ns(payload: &[u8]) -> Option<u64> {
    if payload.len() < 16 {
        return None;
    }
    Some(u64::from_le_bytes([
        payload[8],
        payload[9],
        payload[10],
        payload[11],
        payload[12],
        payload[13],
        payload[14],
        payload[15],
    ]))
}

fn read_u32_le(payload: &[u8], offset: usize) -> Option<u32> {
    if payload.len() < offset + 4 {
        return None;
    }
    Some(u32::from_le_bytes([
        payload[offset],
        payload[offset + 1],
        payload[offset + 2],
        payload[offset + 3],
    ]))
}

/// Extract symbol_id per record type layout.
/// Returns None for record types without symbol_id.
fn extract_symbol_id(rt: u16, payload: &[u8]) -> Option<u32> {
    match rt {
        // seq(8) + ts_ns(8) + symbol_id(4) at offset 16
        RECORD_FILL
        | RECORD_BBO
        | RECORD_ORDER_INSERTED
        | RECORD_ORDER_CANCELLED
        | RECORD_ORDER_DONE
        | RECORD_CONFIG_APPLIED
        | RECORD_MARK_PRICE => read_u32_le(payload, 16),
        // seq(8) + ts_ns(8) + user_id(4) + symbol_id(4) at 20
        RECORD_ORDER_ACCEPTED
        | RECORD_CANCEL_REQUEST
        | RECORD_LIQUIDATION => read_u32_le(payload, 20),
        // seq(8) + ts_ns(8) + user_id(4) + symbol_id(4) at 20
        RECORD_ORDER_RESPONSE => read_u32_le(payload, 20),
        // seq(8) + user_id(4) + symbol_id(4) at 12
        RECORD_ORDER_REQUEST => read_u32_le(payload, 12),
        // no symbol_id in CAUGHT_UP, ORDER_FAILED
        _ => None,
    }
}

/// Extract user_id per record type layout.
/// Returns None for record types without user_id.
fn extract_user_id(rt: u16, payload: &[u8]) -> Option<u32> {
    match rt {
        // seq(8) + ts_ns(8) + symbol_id(4) + user_id(4) at 20
        RECORD_ORDER_INSERTED
        | RECORD_ORDER_CANCELLED
        | RECORD_ORDER_DONE => read_u32_le(payload, 20),
        // seq(8) + ts_ns(8) + taker_user_id(4) at 20 (fill)
        RECORD_FILL => read_u32_le(payload, 20),
        // seq(8) + ts_ns(8) + user_id(4) at 16
        RECORD_ORDER_ACCEPTED
        | RECORD_ORDER_FAILED
        | RECORD_CANCEL_REQUEST
        | RECORD_LIQUIDATION
        | RECORD_ORDER_RESPONSE => read_u32_le(payload, 16),
        // seq(8) + user_id(4) at 8
        RECORD_ORDER_REQUEST => read_u32_le(payload, 8),
        // no user_id in BBO, CONFIG_APPLIED, CAUGHT_UP,
        // MARK_PRICE
        _ => None,
    }
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

/// Wire layout of RECORD_ORDER_REQUEST (rsx_risk::types::OrderRequest).
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct OrderRequestRecord {
    seq: u64,
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
    order_id_hi: u64,
    order_id_lo: u64,
    timestamp_ns: u64,
    side: u8,
    tif: u8,
    reduce_only: u8,
    post_only: u8,
    is_liquidation: u8,
    _pad: [u8; 3],
}

/// Wire layout of RECORD_ORDER_RESPONSE.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct OrderResponseRecord {
    seq: u64,
    ts_ns: u64,
    user_id: u32,
    symbol_id: u32,
    order_id_hi: u64,
    order_id_lo: u64,
    status: u8,
    _pad: [u8; 39],
}

fn oid_hex(hi: u64, lo: u64) -> String {
    format!("{:016x}{:016x}", hi, lo)
}

/// Decode payload bytes into record-specific fields.
/// Returns (text_suffix, json_fields) for the record type.
/// text_suffix uses scale for price/qty display; JSON always raw.
fn decode_payload(
    rt: u16,
    payload: &[u8],
    scale: &DisplayScale,
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
                    scale.px(r.price.0),
                    scale.qty(r.qty.0),
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
                    scale.px(r.bid_px.0),
                    scale.qty(r.bid_qty.0),
                    scale.px(r.ask_px.0),
                    scale.qty(r.ask_qty.0),
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
                    scale.px(r.price.0),
                    scale.qty(r.qty.0),
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
                    scale.qty(r.remaining_qty.0),
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
                    scale.qty(r.filled_qty.0),
                    scale.qty(r.remaining_qty.0),
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
                let cid_str = std::str::from_utf8(&r.cid)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();
                let txt = format!(
                    " sym={} user={} px={} qty={} \
                     side={} oid={} cid={}",
                    r.symbol_id,
                    r.user_id,
                    scale.px(r.price),
                    scale.qty(r.qty),
                    r.side,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                    cid_str,
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
                    "cid": cid_str,
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
                    r.symbol_id,
                    scale.px(r.mark_price.0),
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
            RECORD_ORDER_REQUEST
                if payload.len()
                    >= std::mem::size_of::<
                        OrderRequestRecord,
                    >() =>
            {
                let r: OrderRequestRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} px={} qty={} \
                     side={} oid={}",
                    r.symbol_id,
                    r.user_id,
                    scale.px(r.price),
                    scale.qty(r.qty),
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
            RECORD_ORDER_RESPONSE
                if payload.len()
                    >= std::mem::size_of::<
                        OrderResponseRecord,
                    >() =>
            {
                let r: OrderResponseRecord =
                    std::ptr::read(payload.as_ptr() as *const _);
                let txt = format!(
                    " sym={} user={} status={} oid={}",
                    r.symbol_id,
                    r.user_id,
                    r.status,
                    oid_hex(r.order_id_hi, r.order_id_lo),
                );
                let j = json!({
                    "symbol_id": r.symbol_id,
                    "user_id": r.user_id,
                    "status": r.status,
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
                    scale.qty(r.qty),
                    scale.px(r.price),
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

fn install_ctrlc_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("failed to set ctrl-c handler");
    running
}

fn wal_dump(
    stream_id: u32,
    wal_dir: PathBuf,
    from_seq: u64,
    json: bool,
    stats: bool,
    follow: bool,
    filters: Filters,
    scale: DisplayScale,
) {
    let mut reader = WalReader::open_from_seq(
        stream_id, from_seq, &wal_dir,
    )
    .expect("failed to open wal");

    if stats {
        dump_stats(&mut reader, &filters);
    } else if follow {
        let running = install_ctrlc_handler();
        if json {
            dump_follow_json(
                stream_id, &wal_dir, from_seq, &filters,
                &running,
            );
        } else {
            dump_follow_text(
                stream_id, &wal_dir, from_seq, &filters,
                &running, &scale,
            );
        }
    } else if json {
        dump_json(&mut reader, &filters);
    } else {
        dump_text(&mut reader, &filters, &scale);
    }
}

/// Poll loop for --follow text mode.
/// Re-opens WalReader from last_seq+1 when None is returned.
fn dump_follow_text(
    stream_id: u32,
    wal_dir: &PathBuf,
    from_seq: u64,
    filters: &Filters,
    running: &Arc<AtomicBool>,
    scale: &DisplayScale,
) {
    let mut next_seq = from_seq;
    let mut reader = WalReader::open_from_seq(
        stream_id, next_seq, wal_dir,
    )
    .expect("failed to open wal");

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        match reader.next() {
            Ok(Some(raw)) => {
                let rt = raw.header.record_type;
                if filters.matches(rt, &raw.payload) {
                    let len = raw.header.len;
                    let seq =
                        extract_seq(&raw.payload).unwrap_or(0);
                    let (fields, _) =
                        decode_payload(rt, &raw.payload, scale);
                    println!(
                        "seq={:<8} type={:<18} len={:<4} \
                         crc=0x{:08x}{}",
                        seq,
                        record_name(rt),
                        len,
                        raw.header.crc32,
                        fields,
                    );
                    next_seq = seq + 1;
                }
            }
            Ok(None) => {
                // EOF: sleep then re-open from next_seq
                thread::sleep(Duration::from_millis(100));
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                reader = match WalReader::open_from_seq(
                    stream_id, next_seq, wal_dir,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("wal reopen error: {}", e);
                        thread::sleep(Duration::from_millis(
                            100,
                        ));
                        continue;
                    }
                };
            }
            Err(e) => {
                eprintln!("wal read error: {}", e);
                break;
            }
        }
    }
}

/// Poll loop for --follow JSON mode.
fn dump_follow_json(
    stream_id: u32,
    wal_dir: &PathBuf,
    from_seq: u64,
    filters: &Filters,
    running: &Arc<AtomicBool>,
) {
    let scale = DisplayScale::new(None, None);
    let mut next_seq = from_seq;
    let mut reader = WalReader::open_from_seq(
        stream_id, next_seq, wal_dir,
    )
    .expect("failed to open wal");

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        match reader.next() {
            Ok(Some(raw)) => {
                let rt = raw.header.record_type;
                if filters.matches(rt, &raw.payload) {
                    let len = raw.header.len;
                    let seq =
                        extract_seq(&raw.payload).unwrap_or(0);
                    let (_, fields) =
                        decode_payload(rt, &raw.payload, &scale);
                    let mut obj = json!({
                        "seq": seq,
                        "type": record_name(rt),
                        "len": len,
                        "crc32": format!(
                            "0x{:08x}",
                            raw.header.crc32,
                        ),
                    });
                    if let Value::Object(m) = fields {
                        if let Value::Object(ref mut base) =
                            obj
                        {
                            base.extend(m);
                        }
                    }
                    println!("{}", obj);
                    next_seq = seq + 1;
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(100));
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                reader = match WalReader::open_from_seq(
                    stream_id, next_seq, wal_dir,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("wal reopen error: {}", e);
                        thread::sleep(Duration::from_millis(
                            100,
                        ));
                        continue;
                    }
                };
            }
            Err(e) => {
                eprintln!("wal read error: {}", e);
                break;
            }
        }
    }
}

fn dump_stats(reader: &mut WalReader, filters: &Filters) {
    let mut counts: HashMap<&'static str, u64> = HashMap::new();
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        if !filters.matches(rt, &raw.payload) {
            continue;
        }
        *counts.entry(record_name(rt)).or_insert(0) += 1;
    }
    let mut pairs: Vec<(&str, u64)> =
        counts.into_iter().collect();
    pairs.sort_by_key(|(name, _)| *name);
    let total: u64 = pairs.iter().map(|(_, c)| c).sum();
    for (name, count) in &pairs {
        println!("{:<20} {}", name, count);
    }
    println!("{:<20} {}", "total", total);
}

fn dump_text(
    reader: &mut WalReader,
    filters: &Filters,
    scale: &DisplayScale,
) {
    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        if !filters.matches(rt, &raw.payload) {
            continue;
        }
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        let (fields, _) =
            decode_payload(rt, &raw.payload, scale);

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

fn dump_json(reader: &mut WalReader, filters: &Filters) {
    let mut count: u64 = 0;
    let scale = DisplayScale::new(None, None);
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        if !filters.matches(rt, &raw.payload) {
            continue;
        }
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        let (_, fields) =
            decode_payload(rt, &raw.payload, &scale);

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
        let scale = DisplayScale::new(None, None);
        let (_, fields) = decode_payload(rt, payload, &scale);

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
            record_types,
            symbol,
            user,
            from_ts,
            to_ts,
            stats,
            follow,
            tick_size,
            lot_size,
        } => {
            let filters = Filters::from_args(
                record_types,
                symbol,
                user,
                from_ts,
                to_ts,
            );
            let scale = DisplayScale::new(tick_size, lot_size);
            wal_dump(
                stream_id, wal_dir, from_seq, json, stats,
                follow, filters, scale,
            );
        }
        Commands::Dump { file } => dump_file(file),
    }
}
