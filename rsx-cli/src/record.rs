use rsx_cast::decode_payload;
use rsx_cast::records::CaughtUpRecord;
use rsx_cast::records::RECORD_CAUGHT_UP;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::RawWalRecord;
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
use rsx_messages::CANCEL_REASON_EXPIRY;
use rsx_messages::CANCEL_REASON_OTHER;
use rsx_messages::CANCEL_REASON_POST_ONLY_REJECT;
use rsx_messages::CANCEL_REASON_REDUCE_ONLY;
use rsx_messages::CANCEL_REASON_SYSTEM;
use rsx_messages::CANCEL_REASON_USER_CANCEL;
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
use serde_json::json;
use serde_json::Value;

#[cfg(test)]
#[path = "record_test.rs"]
mod record_test;

/// Divisors for text display of raw i64 prices and quantities.
pub struct DisplayScale {
    tick: f64,
    lot: f64,
}

impl DisplayScale {
    pub fn new(tick_size: Option<f64>, lot_size: Option<f64>) -> Self {
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

/// Every record type name that `--type` accepts and the JSON `type` field
/// carries (besides `UNKNOWN`).
pub const RECORD_NAMES: [&str; 14] = [
    "FILL",
    "BBO",
    "ORDER_INSERTED",
    "ORDER_CANCELLED",
    "ORDER_DONE",
    "CONFIG_APPLIED",
    "CAUGHT_UP",
    "ORDER_ACCEPTED",
    "MARK_PRICE",
    "ORDER_REQUEST",
    "ORDER_RESPONSE",
    "CANCEL_REQUEST",
    "ORDER_FAILED",
    "LIQUIDATION",
];

pub fn name_to_record_type(name: &str) -> Option<u16> {
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
        _ => None,
    }
}

pub fn record_name(record_type: u16) -> &'static str {
    match record_type {
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

/// Wire layout of RECORD_ORDER_REQUEST (`rsx_messages::OrderRequest`), flags as `u8`.
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

fn cancel_reason_text(reason: u8) -> String {
    match reason {
        CANCEL_REASON_USER_CANCEL => "user_cancel".to_string(),
        CANCEL_REASON_REDUCE_ONLY => "reduce_only".to_string(),
        CANCEL_REASON_EXPIRY => "expiry".to_string(),
        CANCEL_REASON_SYSTEM => "system".to_string(),
        CANCEL_REASON_POST_ONLY_REJECT => "post_only_reject".to_string(),
        CANCEL_REASON_OTHER => "other".to_string(),
        _ => format!("unknown_{}", reason),
    }
}

/// The record's own timestamp, if its layout carries one in range.
fn ts_ns(record_type: u16, payload: &[u8]) -> Option<u64> {
    let start = if record_type == RECORD_ORDER_REQUEST {
        48
    } else {
        8
    };
    let bytes = payload.get(start..start + 8)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

/// Text suffix and JSON fields of a payload. The text divides prices and
/// quantities by `scale`; the JSON is always raw. A payload of an unknown
/// type, or shorter than its record, yields an empty suffix and `{}`.
pub fn decode_fields(record_type: u16, payload: &[u8], scale: &DisplayScale) -> (String, Value) {
    decode_known(record_type, payload, scale).unwrap_or_else(|| (String::new(), json!({})))
}

fn decode_known(record_type: u16, payload: &[u8], scale: &DisplayScale) -> Option<(String, Value)> {
    match record_type {
        RECORD_FILL => {
            let r: FillRecord = decode_payload(payload)?;
            let txt = format!(
                " sym={} taker={} maker={} px={} \
                 qty={} side={} oid={}",
                r.symbol_id,
                r.taker_user_id,
                r.maker_user_id,
                scale.px(r.price.0),
                scale.qty(r.qty.0),
                r.taker_side,
                oid_hex(r.taker_order_id_hi, r.taker_order_id_lo),
            );
            let j = json!({
                "symbol_id": r.symbol_id,
                "taker_user_id": r.taker_user_id,
                "maker_user_id": r.maker_user_id,
                "price": r.price.0,
                "qty": r.qty.0,
                "taker_side": r.taker_side,
                "taker_oid": oid_hex(r.taker_order_id_hi, r.taker_order_id_lo),
                "maker_oid": oid_hex(r.maker_order_id_hi, r.maker_order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_BBO => {
            let r: BboRecord = decode_payload(payload)?;
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
            Some((txt, j))
        }
        RECORD_ORDER_INSERTED => {
            let r: OrderInsertedRecord = decode_payload(payload)?;
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
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_ORDER_CANCELLED => {
            let r: OrderCancelledRecord = decode_payload(payload)?;
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
                "reason_text": cancel_reason_text(r.reason),
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_ORDER_DONE => {
            let r: OrderDoneRecord = decode_payload(payload)?;
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
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_CONFIG_APPLIED => {
            let r: ConfigAppliedRecord = decode_payload(payload)?;
            let txt = format!(" sym={} version={}", r.symbol_id, r.config_version);
            let j = json!({
                "symbol_id": r.symbol_id,
                "config_version": r.config_version,
                "effective_at_ms": r.effective_at_ms,
                "applied_at_ns": r.applied_at_ns,
            });
            Some((txt, j))
        }
        RECORD_CAUGHT_UP => {
            let r: CaughtUpRecord = decode_payload(payload)?;
            let txt = format!(" stream={} live_seq={}", r.stream_id, r.live_seq);
            let j = json!({
                "stream_id": r.stream_id,
                "live_seq": r.live_seq,
            });
            Some((txt, j))
        }
        RECORD_ORDER_ACCEPTED => {
            let r: OrderAcceptedRecord = decode_payload(payload)?;
            let cid = std::str::from_utf8(&r.cid)
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
                cid,
            );
            let j = json!({
                "symbol_id": r.symbol_id,
                "user_id": r.user_id,
                "price": r.price,
                "qty": r.qty,
                "side": r.side,
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
                "cid": cid,
            });
            Some((txt, j))
        }
        RECORD_MARK_PRICE => {
            let r: MarkPriceRecord = decode_payload(payload)?;
            let txt = format!(" sym={} mark={}", r.symbol_id, scale.px(r.mark_price.0));
            let j = json!({
                "symbol_id": r.symbol_id,
                "mark_price": r.mark_price.0,
                "source_mask": r.source_mask,
                "source_count": r.source_count,
            });
            Some((txt, j))
        }
        RECORD_CANCEL_REQUEST => {
            let r: CancelRequest = decode_payload(payload)?;
            let txt = format!(
                " sym={} user={} oid={}",
                r.symbol_id,
                r.user_id,
                oid_hex(r.order_id_hi, r.order_id_lo),
            );
            let j = json!({
                "symbol_id": r.symbol_id,
                "user_id": r.user_id,
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_ORDER_FAILED => {
            let r: OrderFailedRecord = decode_payload(payload)?;
            let txt = format!(
                " user={} reason={} oid={}",
                r.user_id,
                r.reason,
                oid_hex(r.order_id_hi, r.order_id_lo),
            );
            let j = json!({
                "user_id": r.user_id,
                "reason": r.reason,
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_ORDER_REQUEST => {
            let r: OrderRequestRecord = decode_payload(payload)?;
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
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_ORDER_RESPONSE => {
            let r: OrderResponseRecord = decode_payload(payload)?;
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
                "oid": oid_hex(r.order_id_hi, r.order_id_lo),
            });
            Some((txt, j))
        }
        RECORD_LIQUIDATION => {
            let r: LiquidationRecord = decode_payload(payload)?;
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
            Some((txt, j))
        }
        _ => None,
    }
}

/// One JSON object: `seq`, `type`, `len` and `crc32` (hex string), then the
/// record's own fields, all raw.
pub fn to_json(raw: &RawWalRecord) -> Value {
    let record_type = raw.header.record_type;
    let (_, fields) = decode_fields(record_type, &raw.payload, &DisplayScale::new(None, None));
    let mut object = json!({
        "seq": extract_seq(&raw.payload).unwrap_or(0),
        "type": record_name(record_type),
        "ts_ns": ts_ns(record_type, &raw.payload),
        "len": raw.header.len,
        "crc32": format!("0x{:08x}", raw.header.crc32),
    });
    if let (Value::Object(base), Value::Object(extra)) = (&mut object, fields) {
        base.extend(extra);
    }
    object
}

/// One text line: `seq`, `type`, `len` and `crc` columns, then the record's
/// own fields scaled by `scale`.
pub fn to_text(raw: &RawWalRecord, scale: &DisplayScale) -> String {
    let record_type = raw.header.record_type;
    let (fields, _) = decode_fields(record_type, &raw.payload, scale);
    format!(
        "seq={:<8} type={:<18} len={:<4} crc=0x{:08x}{}",
        extract_seq(&raw.payload).unwrap_or(0),
        record_name(record_type),
        raw.header.len,
        raw.header.crc32,
        fields,
    )
}
