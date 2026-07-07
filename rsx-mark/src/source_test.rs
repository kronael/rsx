//! Tests for Binance message parsing. The combined-stream endpoint
//! (/stream?streams=) wraps each trade under "data"; the raw
//! single-stream endpoint (/ws/<s>) delivers s/p at the top level.
//! Both must yield a SourcePrice.

use crate::source::handle_binance_msg;
use crate::types::SourcePrice;
use crate::types::SymbolMap;

const SCALE: i64 = 1_000_000;

fn symbol_map() -> SymbolMap {
    let mut m = SymbolMap::new();
    m.insert("PENGUUSDT".to_string(), 10);
    m
}

fn drain(cons: &mut rtrb::Consumer<SourcePrice>) -> Vec<SourcePrice> {
    let mut out = Vec::new();
    while let Ok(p) = cons.pop() {
        out.push(p);
    }
    out
}

#[test]
fn combined_stream_envelope_unwrapped() {
    let (mut prod, mut cons) = rtrb::RingBuffer::<SourcePrice>::new(8);
    let val: serde_json::Value = serde_json::from_str(
        r#"{"stream":"penguusdt@trade",
            "data":{"e":"trade","s":"PENGUUSDT","p":"0.00664200"}}"#,
    )
    .unwrap();
    handle_binance_msg(&val, 0, SCALE, &symbol_map(), &mut prod);
    let got = drain(&mut cons);
    assert_eq!(got.len(), 1, "combined-stream trade must produce a price");
    assert_eq!(got[0].symbol_id, 10);
    assert_eq!(got[0].price, 6642); // 0.006642 * 1e6
}

#[test]
fn raw_stream_flat_still_parsed() {
    let (mut prod, mut cons) = rtrb::RingBuffer::<SourcePrice>::new(8);
    let val: serde_json::Value =
        serde_json::from_str(r#"{"e":"trade","s":"PENGUUSDT","p":"0.00664200"}"#).unwrap();
    handle_binance_msg(&val, 0, SCALE, &symbol_map(), &mut prod);
    let got = drain(&mut cons);
    assert_eq!(got.len(), 1, "raw-stream trade must still parse");
    assert_eq!(got[0].price, 6642);
}

#[test]
fn unknown_symbol_dropped() {
    let (mut prod, mut cons) = rtrb::RingBuffer::<SourcePrice>::new(8);
    let val: serde_json::Value =
        serde_json::from_str(r#"{"data":{"s":"BTCUSDT","p":"60000.00"}}"#).unwrap();
    handle_binance_msg(&val, 0, SCALE, &symbol_map(), &mut prod);
    assert!(drain(&mut cons).is_empty());
}
