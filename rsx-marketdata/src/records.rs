//! Inbound client control frames (subscribe / unsubscribe / heartbeat),
//! parsed from JSON text. The OUTBOUND feed (BBO / snapshot / delta /
//! trade / heartbeat) is protobuf — see `crate::wire`.

use serde_json::Value;

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

pub fn parse_client_frame(text: &str) -> Result<MdFrame, MdParseError> {
    let val: Value = serde_json::from_str(text).map_err(|_| MdParseError::InvalidJson)?;
    let obj = val.as_object().ok_or(MdParseError::InvalidJson)?;
    if obj.len() != 1 {
        return Err(MdParseError::MultipleKeys);
    }
    // SAFETY: obj.len()==1 checked above
    let (key, value) = obj
        .iter()
        .next()
        .expect("INVARIANT: obj has exactly one entry (len==1 checked above)");
    let arr = value.as_array().ok_or(MdParseError::InvalidJson)?;
    match key.as_str() {
        "S" => {
            if arr.len() < 2 {
                return Err(MdParseError::MissingField("S needs 2 fields".into()));
            }
            let sym = arr[0]
                .as_u64()
                .ok_or(MdParseError::InvalidValue("sym".into()))? as u32;
            let ch = arr[1]
                .as_u64()
                .ok_or(MdParseError::InvalidValue("channels".into()))? as u32;
            Ok(MdFrame::Subscribe {
                symbol_id: sym,
                channels: ch,
            })
        }
        "X" => {
            if arr.len() < 2 {
                return Err(MdParseError::MissingField("X needs 2 fields".into()));
            }
            let sym = arr[0]
                .as_u64()
                .ok_or(MdParseError::InvalidValue("sym".into()))? as u32;
            let ch = arr[1]
                .as_u64()
                .ok_or(MdParseError::InvalidValue("channels".into()))? as u32;
            Ok(MdFrame::Unsubscribe {
                symbol_id: sym,
                channels: ch,
            })
        }
        "H" => {
            if arr.is_empty() {
                return Err(MdParseError::MissingField("H needs 1 field".into()));
            }
            let ts = arr[0]
                .as_u64()
                .ok_or(MdParseError::InvalidValue("timestamp_ms".into()))?;
            Ok(MdFrame::Heartbeat { timestamp_ms: ts })
        }
        other => Err(MdParseError::UnknownType(other.into())),
    }
}
