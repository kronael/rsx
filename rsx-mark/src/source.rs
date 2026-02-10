use crate::types::SourcePrice;
use crate::types::SymbolMap;
use futures_util::StreamExt;
use rsx_types::time::time_ns;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_tungstenite::connect_async;

pub trait PriceSource {
    fn start(&self, tx: rtrb::Producer<SourcePrice>);
}

pub struct BinanceSource {
    pub source_id: u8,
    pub ws_url: String,
    pub symbol_map: Arc<SymbolMap>,
    pub reconnect_base_ms: u64,
    pub reconnect_max_ms: u64,
    pub price_scale: i64,
}

pub struct CoinbaseSource {
    pub source_id: u8,
    pub ws_url: String,
    pub symbol_map: Arc<SymbolMap>,
    pub reconnect_base_ms: u64,
    pub reconnect_max_ms: u64,
    pub price_scale: i64,
}

impl PriceSource for BinanceSource {
    fn start(&self, tx: rtrb::Producer<SourcePrice>) {
        run_ws_loop(
            self.ws_url.clone(),
            self.symbol_map.clone(),
            self.source_id,
            self.price_scale,
            self.reconnect_base_ms,
            self.reconnect_max_ms,
            tx,
            handle_binance_msg,
        );
    }
}

impl PriceSource for CoinbaseSource {
    fn start(&self, tx: rtrb::Producer<SourcePrice>) {
        run_ws_loop(
            self.ws_url.clone(),
            self.symbol_map.clone(),
            self.source_id,
            self.price_scale,
            self.reconnect_base_ms,
            self.reconnect_max_ms,
            tx,
            handle_coinbase_msg,
        );
    }
}

fn run_ws_loop<F>(
    ws_url: String,
    symbol_map: Arc<SymbolMap>,
    source_id: u8,
    price_scale: i64,
    base: u64,
    max: u64,
    mut tx: rtrb::Producer<SourcePrice>,
    handler: F,
) where
    F: Fn(
            &serde_json::Value,
            u8,
            i64,
            &SymbolMap,
            &mut rtrb::Producer<SourcePrice>,
        ) + Send
        + 'static,
{
    tokio::spawn(async move {
        let mut backoff = base;
        loop {
            match connect_async(&ws_url).await {
                Ok((mut ws, _)) => {
                    backoff = base;
                    while let Some(msg) = ws.next().await {
                        let msg = match msg {
                            Ok(m) => m,
                            Err(_) => break,
                        };
                        if !msg.is_text() {
                            continue;
                        }
                        let text = msg.to_text().unwrap_or("");
                        if let Ok(val) =
                            serde_json::from_str::<
                                serde_json::Value,
                            >(text)
                        {
                            handler(
                                &val,
                                source_id,
                                price_scale,
                                &symbol_map,
                                &mut tx,
                            );
                        }
                    }
                }
                Err(_) => {}
            }

            tokio::time::sleep(
                Duration::from_millis(backoff),
            )
            .await;
            backoff = (backoff * 2).min(max);
        }
    });
}

fn handle_binance_msg(
    val: &serde_json::Value,
    source_id: u8,
    price_scale: i64,
    symbol_map: &SymbolMap,
    tx: &mut rtrb::Producer<SourcePrice>,
) {
    match val {
        serde_json::Value::Array(arr) => {
            for item in arr {
                handle_binance_msg(
                    item,
                    source_id,
                    price_scale,
                    symbol_map,
                    tx,
                );
            }
        }
        serde_json::Value::Object(map) => {
            let symbol = match map.get("s").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => return,
            };
            let price_str = match map.get("p").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => return,
            };
            let symbol_id = match symbol_map.get(symbol) {
                Some(id) => *id,
                None => return,
            };
            let price = match parse_price(price_str, price_scale) {
                Some(p) => p,
                None => return,
            };
            let _ = tx.push(SourcePrice {
                source_id,
                price,
                timestamp_ns: time_ns(),
                symbol_id,
            });
        }
        _ => {}
    }
}

fn handle_coinbase_msg(
    val: &serde_json::Value,
    source_id: u8,
    price_scale: i64,
    symbol_map: &SymbolMap,
    tx: &mut rtrb::Producer<SourcePrice>,
) {
    let obj = match val.as_object() {
        Some(o) => o,
        None => return,
    };
    let symbol = match obj.get("product_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return,
    };
    let price_str = match obj.get("price").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return,
    };
    let symbol_id = match symbol_map.get(symbol) {
        Some(id) => *id,
        None => return,
    };
    let price = match parse_price(price_str, price_scale) {
        Some(p) => p,
        None => return,
    };
    let _ = tx.push(SourcePrice {
        source_id,
        price,
        timestamp_ns: time_ns(),
        symbol_id,
    });
}

fn parse_price(raw: &str, scale: i64) -> Option<i64> {
    let mut parts = raw.split('.');
    let whole = parts.next().unwrap_or("0");
    let frac = parts.next().unwrap_or("");
    let mut frac_scaled = frac.to_string();
    let scale_digits = scale.to_string().len() - 1;
    if frac_scaled.len() > scale_digits {
        frac_scaled.truncate(scale_digits);
    } else {
        while frac_scaled.len() < scale_digits {
            frac_scaled.push('0');
        }
    }
    let whole_val: i64 = whole.parse().ok()?;
    let frac_val: i64 = if frac_scaled.is_empty() {
        0
    } else {
        frac_scaled.parse().ok()?
    };
    Some(whole_val * scale + frac_val)
}
