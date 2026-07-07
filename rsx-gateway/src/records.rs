use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum WsFrame {
    NewOrder {
        symbol_id: u32,
        side: u8,
        price: i64,
        qty: i64,
        client_order_id: String,
        tif: u8,
        reduce_only: bool,
        post_only: bool,
    },
    Cancel {
        key: CancelKey,
    },
    OrderUpdate {
        order_id: String,
        status: u8,
        filled_qty: i64,
        remaining_qty: i64,
        reason: u8,
    },
    Fill {
        taker_order_id: String,
        maker_order_id: String,
        price: i64,
        qty: i64,
        timestamp_ns: u64,
        fee: i64,
    },
    Error {
        code: u32,
        message: String,
    },
    Heartbeat {
        timestamp_ms: u64,
    },
    Liquidation {
        symbol_id: u32,
        status: u8,
        round: u32,
        side: u8,
        qty: i64,
        price: i64,
        slip_bps: i64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CancelKey {
    ClientOrderId(String),
    OrderId(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    InvalidJson,
    MultipleKeys,
    UnknownType(String),
    MissingField(String),
    InvalidValue(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidJson => {
                write!(f, "invalid json")
            }
            ParseError::MultipleKeys => {
                write!(f, "multiple keys")
            }
            ParseError::UnknownType(k) => {
                write!(f, "unknown type: {}", k)
            }
            ParseError::MissingField(s) => {
                write!(f, "missing field: {}", s)
            }
            ParseError::InvalidValue(s) => {
                write!(f, "invalid value: {}", s)
            }
        }
    }
}

impl std::error::Error for ParseError {}

fn as_u32(v: &Value, field: &str) -> Result<u32, ParseError> {
    v.as_u64()
        .and_then(|n| {
            if n <= u32::MAX as u64 {
                Some(n as u32)
            } else {
                None
            }
        })
        .ok_or_else(|| ParseError::InvalidValue(field.to_string()))
}

fn as_i64(v: &Value, field: &str) -> Result<i64, ParseError> {
    v.as_i64()
        .ok_or_else(|| ParseError::InvalidValue(field.to_string()))
}

fn as_u64(v: &Value, field: &str) -> Result<u64, ParseError> {
    v.as_u64()
        .ok_or_else(|| ParseError::InvalidValue(field.to_string()))
}

fn as_str<'a>(v: &'a Value, field: &str) -> Result<&'a str, ParseError> {
    v.as_str()
        .ok_or_else(|| ParseError::InvalidValue(field.to_string()))
}

fn as_u8(v: &Value, field: &str) -> Result<u8, ParseError> {
    v.as_u64()
        .and_then(|n| if n <= 255 { Some(n as u8) } else { None })
        .ok_or_else(|| ParseError::InvalidValue(field.to_string()))
}

fn arr_get<'a>(arr: &'a [Value], idx: usize, field: &str) -> Result<&'a Value, ParseError> {
    arr.get(idx)
        .ok_or_else(|| ParseError::MissingField(field.to_string()))
}

pub fn parse(text: &str) -> Result<WsFrame, ParseError> {
    let val: Value = serde_json::from_str(text).map_err(|_| ParseError::InvalidJson)?;
    let obj = val.as_object().ok_or(ParseError::InvalidJson)?;
    if obj.len() != 1 {
        return Err(ParseError::MultipleKeys);
    }
    // SAFETY: obj.len()==1 checked above
    let (key, value) = obj
        .iter()
        .next()
        .expect("INVARIANT: obj has exactly one entry (len==1 checked above)");

    // Validate key is alphabetic
    if !key.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ParseError::UnknownType(key.clone()));
    }

    let arr = value.as_array().ok_or(ParseError::InvalidJson)?;

    match key.as_str() {
        "N" => parse_new_order(arr),
        "C" => parse_cancel(arr),
        "U" => parse_order_update(arr),
        "F" => parse_fill(arr),
        "E" => parse_error(arr),
        "H" => parse_heartbeat(arr),
        "Q" => parse_liquidation(arr),
        other => Err(ParseError::UnknownType(other.to_string())),
    }
}

fn parse_new_order(arr: &[Value]) -> Result<WsFrame, ParseError> {
    if arr.len() < 6 {
        return Err(ParseError::MissingField(
            "N requires at least 6 fields".to_string(),
        ));
    }
    let symbol_id = as_u32(&arr[0], "sym")?;
    let side = as_u8(&arr[1], "side")?;
    if side > 1 {
        return Err(ParseError::InvalidValue("side must be 0 or 1".to_string()));
    }
    let price = as_i64(&arr[2], "px")?;
    let qty = as_i64(&arr[3], "qty")?;
    let cid = as_str(&arr[4], "cid")?.to_string();
    if cid.is_empty() || cid.len() > 20 {
        return Err(ParseError::InvalidValue(
            "cid must be 1-20 chars".to_string(),
        ));
    }
    let tif = as_u8(&arr[5], "tif")?;
    if tif > 2 {
        return Err(ParseError::InvalidValue(
            "tif must be 0, 1, or 2".to_string(),
        ));
    }
    let reduce_only = if arr.len() > 6 {
        as_u8(&arr[6], "ro")? != 0
    } else {
        false
    };
    let post_only = if arr.len() > 7 {
        as_u8(&arr[7], "po")? != 0
    } else {
        false
    };
    Ok(WsFrame::NewOrder {
        symbol_id,
        side,
        price,
        qty,
        client_order_id: cid,
        tif,
        reduce_only,
        post_only,
    })
}

fn parse_cancel(arr: &[Value]) -> Result<WsFrame, ParseError> {
    let v = arr_get(arr, 0, "cid_or_oid")?;
    let s = as_str(v, "cid_or_oid")?.to_string();
    let key = if s.len() == 20 {
        CancelKey::ClientOrderId(s)
    } else if s.len() == 32 {
        CancelKey::OrderId(s)
    } else {
        return Err(ParseError::InvalidValue(
            "cancel key must be 20 (cid) or 32 (oid) chars".to_string(),
        ));
    };
    Ok(WsFrame::Cancel { key })
}

fn parse_order_update(arr: &[Value]) -> Result<WsFrame, ParseError> {
    if arr.len() < 5 {
        return Err(ParseError::MissingField("U requires 5 fields".to_string()));
    }
    let oid = as_str(&arr[0], "oid")?.to_string();
    if oid.len() != 32 {
        return Err(ParseError::InvalidValue("oid must be 32 chars".to_string()));
    }
    let status = as_u8(&arr[1], "status")?;
    if status > 3 {
        return Err(ParseError::InvalidValue("status must be 0-3".to_string()));
    }
    let filled_qty = as_i64(&arr[2], "filled")?;
    let remaining_qty = as_i64(&arr[3], "remaining")?;
    let reason = as_u8(&arr[4], "reason")?;
    if reason > 12 {
        return Err(ParseError::InvalidValue("reason must be 0-12".to_string()));
    }
    Ok(WsFrame::OrderUpdate {
        order_id: oid,
        status,
        filled_qty,
        remaining_qty,
        reason,
    })
}

fn parse_fill(arr: &[Value]) -> Result<WsFrame, ParseError> {
    if arr.len() < 6 {
        return Err(ParseError::MissingField("F requires 6 fields".to_string()));
    }
    let taker = as_str(&arr[0], "taker_oid")?.to_string();
    let maker = as_str(&arr[1], "maker_oid")?.to_string();
    let price = as_i64(&arr[2], "px")?;
    let qty = as_i64(&arr[3], "qty")?;
    let ts = as_u64(&arr[4], "ts")?;
    let fee = as_i64(&arr[5], "fee")?;
    Ok(WsFrame::Fill {
        taker_order_id: taker,
        maker_order_id: maker,
        price,
        qty,
        timestamp_ns: ts,
        fee,
    })
}

fn parse_error(arr: &[Value]) -> Result<WsFrame, ParseError> {
    if arr.len() < 2 {
        return Err(ParseError::MissingField("E requires 2 fields".to_string()));
    }
    let code = as_u32(&arr[0], "code")?;
    let msg = as_str(&arr[1], "msg")?.to_string();
    Ok(WsFrame::Error { code, message: msg })
}

fn parse_heartbeat(arr: &[Value]) -> Result<WsFrame, ParseError> {
    let v = arr_get(arr, 0, "ts")?;
    let ts = as_u64(v, "ts")?;
    Ok(WsFrame::Heartbeat { timestamp_ms: ts })
}

fn parse_liquidation(arr: &[Value]) -> Result<WsFrame, ParseError> {
    if arr.len() < 7 {
        return Err(ParseError::MissingField("Q requires 7 fields".to_string()));
    }
    let symbol_id = as_u32(&arr[0], "sym")?;
    let status = as_u8(&arr[1], "status")?;
    if status > 4 {
        return Err(ParseError::InvalidValue(
            "liquidation status must be 0-4".to_string(),
        ));
    }
    let round = as_u32(&arr[2], "round")?;
    let side = as_u8(&arr[3], "side")?;
    if side > 1 {
        return Err(ParseError::InvalidValue("side must be 0 or 1".to_string()));
    }
    let qty = as_i64(&arr[4], "qty")?;
    let price = as_i64(&arr[5], "price")?;
    let slip_bps = as_i64(&arr[6], "slip_bps")?;
    Ok(WsFrame::Liquidation {
        symbol_id,
        status,
        round,
        side,
        qty,
        price,
        slip_bps,
    })
}

pub fn serialize(frame: &WsFrame) -> String {
    match frame {
        WsFrame::NewOrder {
            symbol_id,
            side,
            price,
            qty,
            client_order_id,
            tif,
            reduce_only,
            post_only,
        } => {
            let ro = if *reduce_only { 1 } else { 0 };
            let po = if *post_only { 1 } else { 0 };
            format!(
                "{{\"N\":[{},{},{},{},\"{}\",{},{},{}]}}",
                symbol_id, side, price, qty, client_order_id, tif, ro, po,
            )
        }
        WsFrame::Cancel { key } => {
            let s = match key {
                CancelKey::ClientOrderId(s) => s,
                CancelKey::OrderId(s) => s,
            };
            format!("{{\"C\":[\"{}\"]}}", s)
        }
        WsFrame::OrderUpdate {
            order_id,
            status,
            filled_qty,
            remaining_qty,
            reason,
        } => {
            format!(
                "{{\"U\":[\"{}\",{},{},{},{}]}}",
                order_id, status, filled_qty, remaining_qty, reason,
            )
        }
        WsFrame::Fill {
            taker_order_id,
            maker_order_id,
            price,
            qty,
            timestamp_ns,
            fee,
        } => {
            format!(
                "{{\"F\":[\"{}\",\"{}\",{},{},{},{}]}}",
                taker_order_id, maker_order_id, price, qty, timestamp_ns, fee,
            )
        }
        WsFrame::Error { code, message } => {
            format!("{{\"E\":[{},\"{}\"]}}", code, message,)
        }
        WsFrame::Heartbeat { timestamp_ms } => {
            format!("{{\"H\":[{}]}}", timestamp_ms)
        }
        WsFrame::Liquidation {
            symbol_id,
            status,
            round,
            side,
            qty,
            price,
            slip_bps,
        } => {
            format!(
                "{{\"Q\":[{},{},{},{},{},{},{}]}}",
                symbol_id, status, round, side, qty, price, slip_bps,
            )
        }
    }
}
