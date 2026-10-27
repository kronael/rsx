use crate::output::Failure;
use rsx_risk::config::load_symbol_configs;
use rsx_risk::db::connect;
use rsx_types::client_order::one_usd_notional;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[cfg(test)]
#[path = "symbols_test.rs"]
mod symbols_test;

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// One row of the gateway's `/v1/symbols`: tick and lot in raw units and the
/// decimals that turn a human price or quantity into them.
#[derive(Debug, Deserialize, PartialEq)]
pub struct Symbol {
    #[serde(rename = "id")]
    pub symbol_id: u32,
    pub tick_size: i64,
    pub lot_size: i64,
    pub price_decimals: u32,
    pub qty_decimals: u32,
}

#[derive(Deserialize)]
struct Catalog {
    symbols: Vec<Symbol>,
}

impl Symbol {
    /// 1 USD in raw `price * qty` units, from the decimals: the minimum
    /// notional the preset symbols carry. Risk enforces the symbol's
    /// configured minimum, which `--db` reads.
    pub fn default_min_notional(&self) -> i64 {
        one_usd_notional(self.price_decimals, self.qty_decimals)
    }
}

/// Fetches the symbol catalog from the gateway behind the WebSocket `url`;
/// the gateway answers `GET /v1/symbols` on its order port.
pub async fn fetch(url: &str) -> Result<Vec<Symbol>, Failure> {
    let address = http_address(url)?;
    let body = match timeout(FETCH_TIMEOUT, get_symbols(&address)).await {
        Ok(body) => body?,
        Err(_) => {
            return Err(Failure::runtime(format!(
                "no /v1/symbols answer from {address} within {} s",
                FETCH_TIMEOUT.as_secs()
            )))
        }
    };
    let catalog: Catalog = serde_json::from_str(&body).map_err(|error| {
        Failure::runtime(format!("bad /v1/symbols body from {address}: {error}"))
    })?;
    Ok(catalog.symbols)
}

/// The minimum order notional (raw `price * qty`) risk enforces for each of
/// `symbol_ids`, from the active `symbol_config_schedule` rows at
/// `database_url`. A symbol without an active row is a `Runtime` failure.
pub async fn fetch_min_notionals(
    database_url: &str,
    symbol_ids: &[u32],
) -> Result<HashMap<u32, i64>, Failure> {
    let client = connect(database_url)
        .await
        .map_err(|error| Failure::runtime(format!("failed to connect to postgres: {error}")))?;
    let limit = symbol_ids.iter().max().map_or(0, |max| *max as usize + 1);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let configs = load_symbol_configs(&client, limit, now_ms)
        .await
        .map_err(Failure::runtime)?;
    let mut min_notionals = HashMap::new();
    for symbol_id in symbol_ids {
        let Some(config) = configs.iter().find(|config| config.symbol_id == *symbol_id) else {
            return Err(Failure::runtime(format!(
                "symbol {symbol_id} has no active config in postgres"
            )));
        };
        min_notionals.insert(*symbol_id, config.params.min_notional);
    }
    Ok(min_notionals)
}

/// The symbol with `symbol_id`, or a `Misuse` failure naming the symbols the
/// gateway serves.
pub fn find(symbols: &[Symbol], symbol_id: u32) -> Result<&Symbol, Failure> {
    symbols
        .iter()
        .find(|symbol| symbol.symbol_id == symbol_id)
        .ok_or_else(|| {
            let known: Vec<String> = symbols
                .iter()
                .map(|symbol| symbol.symbol_id.to_string())
                .collect();
            Failure::misuse(format!(
                "symbol {symbol_id} is not served by the gateway (symbols: {})",
                known.join(", ")
            ))
        })
}

/// Converts a non-negative decimal `text` to raw units with `decimals`
/// fractional digits. Digits beyond `decimals` must be zeros; a value that
/// does not fit an `i64` is an error.
pub fn to_raw(text: &str, decimals: u32) -> Result<i64, String> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let is_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !is_digits(whole) || !is_digits(fraction) {
        return Err(format!("{text:?} is not a non-negative decimal number"));
    }
    let fraction = fraction.trim_end_matches('0');
    let width = decimals as usize;
    if fraction.len() > width {
        return Err(format!("{text} has more than {decimals} decimals"));
    }
    format!("{whole}{fraction:0<width$}")
        .parse::<i64>()
        .map_err(|_| format!("{text} does not fit 64-bit raw units"))
}

fn http_address(url: &str) -> Result<String, Failure> {
    let rest = url.strip_prefix("ws://").ok_or_else(|| {
        Failure::misuse(format!(
            "gateway url {url} must start with ws:// (TLS terminates at a proxy)"
        ))
    })?;
    let host = rest.split('/').next().unwrap_or(rest);
    if host.contains(':') {
        Ok(host.to_string())
    } else {
        Ok(format!("{host}:80"))
    }
}

async fn get_symbols(address: &str) -> Result<String, Failure> {
    let mut stream = TcpStream::connect(address)
        .await
        .map_err(|error| Failure::runtime(format!("failed to connect to {address}: {error}")))?;
    let request =
        format!("GET /v1/symbols HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| Failure::runtime(format!("failed to send to {address}: {error}")))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|error| Failure::runtime(format!("read from {address} failed: {error}")))?;
    let response = String::from_utf8_lossy(&response);
    let Some((head, body)) = response.split_once("\r\n\r\n") else {
        return Err(Failure::runtime(format!(
            "no HTTP response from {address} for /v1/symbols"
        )));
    };
    let status_line = head.lines().next().unwrap_or("");
    if status_line.split_whitespace().nth(1) != Some("200") {
        return Err(Failure::runtime(format!(
            "{address} answered /v1/symbols with {status_line:?}"
        )));
    }
    Ok(body.to_string())
}
