use crate::types::SourcePrice;
use crate::types::SymbolMap;
use futures_util::StreamExt;
use rsx_types::time::time_ns;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
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

#[allow(clippy::too_many_arguments)]
/// ±20% jitter multiplier — avoids adding a rand dep.
fn jitter_factor() -> f64 {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    0.8 + 0.4 * ((ns % 1000) as f64 / 1000.0)
}

#[allow(clippy::too_many_arguments)]
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
    // Hard retry budget; reset on each successful connection.
    const MAX_RETRIES: u32 = 20;

    tokio::spawn(async move {
        let mut backoff = base;
        let mut consec_errors: u32 = 0;
        loop {
            tracing::info!(
                "ws connecting to {}", ws_url,
            );
            match connect_async(&ws_url).await {
                Ok((mut ws, _)) => {
                    // Connected — reset budget and backoff.
                    tracing::info!(
                        "ws connected to {}", ws_url,
                    );
                    backoff = base;
                    consec_errors = 0;
                    while let Some(msg) = ws.next().await {
                        let msg = match msg {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!(
                                    "ws read error: {e}",
                                );
                                break;
                            }
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
                Err(e) => {
                    tracing::warn!(
                        "ws connect error: {e}",
                    );
                    consec_errors += 1;
                }
            }

            if consec_errors > MAX_RETRIES {
                tracing::error!(
                    "BLOCKED: source_id={} exhausted {} \
                     reconnect attempts; stopping",
                    source_id,
                    MAX_RETRIES,
                );
                return;
            }

            let sleep_ms = (backoff as f64
                * jitter_factor()) as u64;
            tokio::time::sleep(
                Duration::from_millis(sleep_ms),
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
            tracing::debug!(
                "binance price: sym={} px={}",
                symbol, price,
            );
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

fn is_power_of_10(n: i64) -> bool {
    if n <= 0 {
        return false;
    }
    let mut v = n;
    while v > 1 {
        if v % 10 != 0 {
            return false;
        }
        v /= 10;
    }
    true
}

fn parse_price(raw: &str, scale: i64) -> Option<i64> {
    if !is_power_of_10(scale) {
        return None;
    }
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
    let base = whole_val.checked_mul(scale)?;
    if whole_val < 0 {
        base.checked_sub(frac_val)
    } else {
        base.checked_add(frac_val)
    }
}
