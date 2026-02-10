use crate::types::BboUpdate;
use crate::types::L2Delta;
use crate::types::L2Level;
use crate::types::L2Snapshot;
use crate::types::TradeEvent;
use serde_json::Value;

pub fn serialize_bbo(bbo: &BboUpdate) -> String {
    format!(
        "{{\"BBO\":[{},{},{},{},{},{},{},{},{}]}}",
        bbo.symbol_id, bbo.bid_px, bbo.bid_qty,
        bbo.bid_count, bbo.ask_px, bbo.ask_qty,
        bbo.ask_count, bbo.timestamp_ns, bbo.seq,
    )
}

pub fn serialize_l2_snapshot(snap: &L2Snapshot) -> String {
    let fmt_levels = |levels: &[L2Level]| {
        let parts: Vec<String> = levels
            .iter()
            .map(|l| {
                format!("[{},{},{}]", l.price, l.qty, l.count)
            })
            .collect();
        format!("[{}]", parts.join(","))
    };
    format!(
        "{{\"B\":[{},{},{},{},{}]}}",
        snap.symbol_id,
        fmt_levels(&snap.bids),
        fmt_levels(&snap.asks),
        snap.timestamp_ns,
        snap.seq,
    )
}

pub fn serialize_l2_delta(delta: &L2Delta) -> String {
    format!(
        "{{\"D\":[{},{},{},{},{},{},{}]}}",
        delta.symbol_id, delta.side, delta.price,
        delta.qty, delta.count,
        delta.timestamp_ns, delta.seq,
    )
}

pub fn serialize_trade(trade: &TradeEvent) -> String {
    format!(
        "{{\"T\":[{},{},{},{},{},{}]}}",
        trade.symbol_id, trade.price, trade.qty,
        trade.taker_side, trade.timestamp_ns, trade.seq,
    )
}

#[derive(Debug, Clone, PartialEq)]
pub enum MdFrame {
    Subscribe { symbol_id: u32, channels: u32 },
    Unsubscribe { symbol_id: u32, channels: u32 },
    Heartbeat { timestamp_ms: u64 },
}

#[derive(Debug)]
pub enum MdParseError {
    InvalidJson,
    MultipleKeys,
    UnknownType(String),
    MissingField(String),
    InvalidValue(String),
}

pub fn parse_client_frame(
    text: &str,
) -> Result<MdFrame, MdParseError> {
    let val: Value = serde_json::from_str(text)
        .map_err(|_| MdParseError::InvalidJson)?;
    let obj = val
        .as_object()
        .ok_or(MdParseError::InvalidJson)?;
    if obj.len() != 1 {
        return Err(MdParseError::MultipleKeys);
    }
    let (key, value) = obj.iter().next().unwrap();
    let arr = value
        .as_array()
        .ok_or(MdParseError::InvalidJson)?;
    match key.as_str() {
        "S" => {
            if arr.len() < 2 {
                return Err(MdParseError::MissingField(
                    "S needs 2 fields".into(),
                ));
            }
            let sym = arr[0]
                .as_u64()
                .ok_or(MdParseError::InvalidValue(
                    "sym".into(),
                ))? as u32;
            let ch = arr[1]
                .as_u64()
                .ok_or(MdParseError::InvalidValue(
                    "channels".into(),
                ))? as u32;
            Ok(MdFrame::Subscribe {
                symbol_id: sym,
                channels: ch,
            })
        }
        "X" => {
            if arr.len() < 2 {
                return Err(MdParseError::MissingField(
                    "X needs 2 fields".into(),
                ));
            }
            let sym = arr[0]
                .as_u64()
                .ok_or(MdParseError::InvalidValue(
                    "sym".into(),
                ))? as u32;
            let ch = arr[1]
                .as_u64()
                .ok_or(MdParseError::InvalidValue(
                    "channels".into(),
                ))? as u32;
            Ok(MdFrame::Unsubscribe {
                symbol_id: sym,
                channels: ch,
            })
        }
        "H" => {
            if arr.is_empty() {
                return Err(MdParseError::MissingField(
                    "H needs 1 field".into(),
                ));
            }
            let ts = arr[0]
                .as_u64()
                .ok_or(MdParseError::InvalidValue(
                    "timestamp_ms".into(),
                ))?;
            Ok(MdFrame::Heartbeat { timestamp_ms: ts })
        }
        other => {
            Err(MdParseError::UnknownType(other.into()))
        }
    }
}
