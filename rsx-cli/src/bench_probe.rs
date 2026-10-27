//! bench-probe binary: native Rust E2E latency probe. See `--help` for usage.

use clap::Parser;
use futures_util::SinkExt;
use futures_util::StreamExt;
use rsx_messages::mint_jwt;
use rsx_messages::Claims;
use rsx_types::client_order::min_order_qty;
use rsx_types::client_order::one_usd_notional;
use rsx_types::client_order::CidMint;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser, Debug)]
#[command(name = "bench-probe", about = "Native Rust E2E latency probe")]
struct Args {
    /// Gateway WS URL, e.g. ws://127.0.0.1:8080
    #[arg(long, default_value = "ws://127.0.0.1:8080")]
    gateway: String,
    /// Playground HTTP URL (for /api/book and baseline read).
    #[arg(long, default_value = "http://127.0.0.1:49171")]
    playground: String,
    /// Symbol id to trade against.
    #[arg(long, default_value_t = 10)]
    symbol_id: u32,
    /// JWT signing secret (HS256). Defaults to env
    /// RSX_GW_JWT_SECRET when unset.
    #[arg(long)]
    jwt_secret: Option<String>,
    /// Number of probe orders to send.
    #[arg(long, default_value_t = 2000)]
    n: usize,
    /// Number of warmup orders to discard before measuring.
    #[arg(long, default_value_t = 50)]
    warmup: usize,
    /// Per-probe timeout in seconds.
    #[arg(long, default_value_t = 2.0)]
    timeout_s: f64,
    /// User id to mint the JWT for.
    #[arg(long, default_value_t = 1)]
    user_id: u32,
    /// Lot size for the order qty (raw i64). Defaults to the
    /// symbol's lot_size in the playground's /v1/symbols catalog.
    #[arg(long)]
    lot_size: Option<i64>,
    /// Minimum order notional (raw price x qty units). Defaults to
    /// the catalog's min_notional, or 1 USD from the symbol's
    /// decimals when the catalog omits it.
    #[arg(long)]
    min_notional: Option<i64>,
    /// Path to bench-e2e-latest.json for side-by-side compare.
    #[arg(long, default_value = "bench-e2e-latest.json")]
    baseline: PathBuf,
}

#[derive(Deserialize, Debug)]
struct BookSnap {
    asks: Vec<BookLevel>,
}

#[derive(Deserialize, Debug)]
struct BookLevel {
    px: i64,
}

/// One row of the playground's /v1/symbols catalog.
#[derive(Deserialize, Debug)]
struct SymbolRow {
    id: u32,
    lot_size: i64,
    #[serde(default)]
    price_decimals: u32,
    #[serde(default)]
    qty_decimals: u32,
    /// Absent while the catalog's config source is down.
    min_notional: Option<i64>,
}

#[derive(Deserialize, Debug)]
struct SymbolCatalog {
    symbols: Vec<SymbolRow>,
}

impl SymbolRow {
    /// The row's minimum order notional in raw price x qty units, else
    /// 1 USD from its decimals (10^(price_decimals + qty_decimals)).
    fn minimum_notional(&self) -> i64 {
        match self.min_notional {
            Some(value) if value > 0 => value,
            _ => one_usd_notional(self.price_decimals, self.qty_decimals),
        }
    }
}

/// Lot size, minimum notional and order quantity of a probe at `price`:
/// the flags win over the catalog row, and the quantity is the smallest lot
/// multiple whose notional at `price` reaches the minimum.
fn probe_qty(
    row: &SymbolRow,
    lot_size: Option<i64>,
    min_notional: Option<i64>,
    price: i64,
) -> (i64, i64, i64) {
    let lot_size = lot_size.unwrap_or(row.lot_size);
    let min_notional = min_notional.unwrap_or_else(|| row.minimum_notional());
    (
        lot_size,
        min_notional,
        min_order_qty(min_notional, price, lot_size),
    )
}

async fn fetch_symbol(playground: &str, symbol_id: u32) -> Result<SymbolRow, String> {
    let url = format!("{}/v1/symbols", playground.trim_end_matches('/'));
    let body = http_get(&url).await?;
    let catalog: SymbolCatalog = serde_json::from_slice(&body).map_err(|e| {
        format!(
            "parse symbols: {} body={}",
            e,
            String::from_utf8_lossy(&body)
        )
    })?;
    catalog
        .symbols
        .into_iter()
        .find(|row| row.id == symbol_id)
        .ok_or_else(|| format!("symbol {} not in the catalog", symbol_id))
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("Error: {}", msg);
    exit(1);
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mint_user_jwt(secret: &str, user_id: u32) -> String {
    let claims = Claims::for_user(user_id, unix_now_secs() + 300);
    mint_jwt(&claims, secret).unwrap_or_else(|e| die(e))
}

/// Minimal blocking HTTP GET that returns the body bytes.
/// Only supports http:// (loopback). Used to read /api/book.
async fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let stripped = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// supported: {}", url))?;
    let slash = stripped.find('/').unwrap_or(stripped.len());
    let host = &stripped[..slash];
    let path = if slash < stripped.len() {
        &stripped[slash..]
    } else {
        "/"
    };
    // Default port 80 unless host has explicit port.
    let addr = if host.contains(':') {
        host.to_string()
    } else {
        format!("{}:80", host)
    };
    let mut sock = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("connect {}: {}", addr, e))?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host,
    );
    sock.write_all(req.as_bytes())
        .await
        .map_err(|e| format!("write: {}", e))?;
    let mut buf = Vec::with_capacity(8192);
    sock.read_to_end(&mut buf)
        .await
        .map_err(|e| format!("read: {}", e))?;
    let sep = b"\r\n\r\n";
    let idx = buf
        .windows(4)
        .position(|w| w == sep)
        .ok_or_else(|| "no http header terminator".to_string())?;
    Ok(buf[idx + 4..].to_vec())
}

async fn fetch_best_ask(playground: &str, symbol_id: u32) -> Result<i64, String> {
    let url = format!(
        "{}/api/book/{}",
        playground.trim_end_matches('/'),
        symbol_id,
    );
    let body = http_get(&url).await?;
    let snap: BookSnap = serde_json::from_slice(&body)
        .map_err(|e| format!("parse book: {} body={}", e, String::from_utf8_lossy(&body)))?;
    snap.asks
        .first()
        .map(|a| a.px)
        .ok_or_else(|| "no asks in book (maker idle?)".to_string())
}

fn read_baseline_e2e(path: &PathBuf) -> Option<(f64, f64, u64)> {
    let bytes = fs::read(path).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let e2e = v.get("e2e_us")?;
    let p50 = e2e.get("p50")?.as_f64()?;
    let p99 = e2e.get("p99")?.as_f64()?;
    let n = e2e.get("n")?.as_u64()?;
    Some((p50, p99, n))
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64) * p) as usize;
    let idx = idx.min(sorted.len() - 1);
    sorted[idx]
}

/// Send one probe order and wait for the matching fill.
/// Returns Some(us) on success, None on skip/timeout.
async fn one_probe(
    ws_url: &str,
    token: &str,
    symbol_id: u32,
    cross_px: i64,
    qty: i64,
    cid: &str,
    timeout: Duration,
) -> Result<(u64, u32), String> {
    let mut req: Request<()> = ws_url
        .into_client_request()
        .map_err(|e| format!("bad ws url: {}", e))?;
    let headers = req.headers_mut();
    headers.insert(
        "Authorization",
        format!("Bearer {}", token)
            .parse()
            .map_err(|e| format!("bad auth hdr: {}", e))?,
    );
    headers.insert(
        "Sec-WebSocket-Key",
        generate_key()
            .parse()
            .map_err(|e| format!("ws key: {}", e))?,
    );
    headers.insert("Sec-WebSocket-Version", "13".parse().unwrap());
    headers.insert("Connection", "Upgrade".parse().unwrap());
    headers.insert("Upgrade", "websocket".parse().unwrap());

    // Resolve host:port from the URL.
    let url_no_scheme = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .ok_or_else(|| format!("bad ws url: {}", ws_url))?;
    let host_part = url_no_scheme.split('/').next().unwrap_or(url_no_scheme);
    let addr = if host_part.contains(':') {
        host_part.to_string()
    } else {
        format!("{}:80", host_part)
    };
    let sock = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("ws connect: {}", e))?;

    let (mut ws, _resp) = client_async(req, sock)
        .await
        .map_err(|e| format!("ws handshake: {}", e))?;

    let order = serde_json::json!({
        "N": [symbol_id, 0, cross_px, qty, cid, 0],
    });
    let t0 = Instant::now();
    ws.send(Message::Text(order.to_string()))
        .await
        .map_err(|e| format!("ws send: {}", e))?;

    let deadline = Instant::now() + timeout;
    let mut probe_oid: Option<String> = None;
    let mut skipped: u32 = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("timeout (skipped={})", skipped));
        }
        let recv = tokio::time::timeout(remaining, ws.next()).await;
        let msg = match recv {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => return Err(format!("ws err: {}", e)),
            Ok(None) => return Err("ws closed".to_string()),
            Err(_) => return Err(format!("timeout (skipped={})", skipped)),
        };
        let text = match msg {
            Message::Text(t) => t,
            _ => continue,
        };
        let frame: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(u) = frame.get("U").and_then(|v| v.as_array()) {
            if probe_oid.is_none() {
                if let Some(oid) = u.first().and_then(|v| v.as_str()) {
                    if oid.len() == 32 {
                        probe_oid = Some(oid.to_string());
                    }
                }
            }
            continue;
        }
        if let Some(f) = frame.get("F").and_then(|v| v.as_array()) {
            let taker_oid = f.first().and_then(|v| v.as_str());
            if let (Some(want), Some(got)) = (probe_oid.as_deref(), taker_oid) {
                if want == got {
                    let elapsed_ns = t0.elapsed().as_nanos();
                    let elapsed_us = (elapsed_ns / 1000).max(1) as u64;
                    // SAFETY: best-effort close; we have
                    // the measurement and are returning
                    // regardless of socket close outcome.
                    let _close = ws.close(None).await;
                    return Ok((elapsed_us, skipped));
                }
            }
            skipped += 1;
            continue;
        }
        if let Some(err) = frame.get("E") {
            return Err(format!("gateway error frame: {}", err));
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();
    let secret = args
        .jwt_secret
        .clone()
        .or_else(|| std::env::var("RSX_GW_JWT_SECRET").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| die("missing --jwt-secret or RSX_GW_JWT_SECRET"));

    println!(
        "[bench-probe] gateway={} playground={} symbol={} n={} warmup={}",
        args.gateway, args.playground, args.symbol_id, args.n, args.warmup,
    );

    let best_ask = fetch_best_ask(&args.playground, args.symbol_id)
        .await
        .unwrap_or_else(|e| die(format!("fetch best ask: {}", e)));
    // 1% above ask, same heuristic as the Python probe.
    let cross_px = best_ask * 101 / 100;
    println!("[bench-probe] best_ask={} cross_px={}", best_ask, cross_px);

    // Risk rejects an order whose price x qty is under the symbol's
    // minimum, so the probe sends the smallest lot multiple that reaches
    // it at cross_px.
    let row = fetch_symbol(&args.playground, args.symbol_id)
        .await
        .unwrap_or_else(|e| die(format!("fetch symbol: {}", e)));
    let (lot_size, min_notional, qty) = probe_qty(&row, args.lot_size, args.min_notional, cross_px);
    println!(
        "[bench-probe] lot_size={} min_notional={} qty={}",
        lot_size, min_notional, qty,
    );

    let timeout = Duration::from_secs_f64(args.timeout_s);

    let mut cids = CidMint::new();

    // Warmup
    for i in 0..args.warmup {
        let cid = cids.next();
        let token = mint_user_jwt(&secret, args.user_id);
        if let Err(e) = one_probe(
            &args.gateway,
            &token,
            args.symbol_id,
            cross_px,
            qty,
            &cid,
            timeout,
        )
        .await
        {
            eprintln!("[bench-probe] warmup {} failed: {}", i, e);
        }
    }

    // Measurement
    let mut samples: Vec<u64> = Vec::with_capacity(args.n);
    let mut skipped_total: u64 = 0;
    let mut failed: u64 = 0;
    for i in 0..args.n {
        let cid = cids.next();
        let token = mint_user_jwt(&secret, args.user_id);
        match one_probe(
            &args.gateway,
            &token,
            args.symbol_id,
            cross_px,
            qty,
            &cid,
            timeout,
        )
        .await
        {
            Ok((us, skipped)) => {
                samples.push(us);
                skipped_total += skipped as u64;
            }
            Err(e) => {
                failed += 1;
                if failed <= 5 {
                    eprintln!("[bench-probe] probe {} failed: {}", i, e);
                }
            }
        }
    }

    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let p99 = percentile(&samples, 0.99);
    let min = samples.first().copied().unwrap_or(0);
    let max = samples.last().copied().unwrap_or(0);
    let n = samples.len();

    let baseline = read_baseline_e2e(&args.baseline);

    println!();
    println!("=== bench-probe results ===");
    println!("count        = {}", n);
    println!("failed       = {}", failed);
    println!("skipped_fills= {}", skipped_total);
    println!("p50_us       = {}", p50);
    println!("p95_us       = {}", p95);
    println!("p99_us       = {}", p99);
    println!("min_us       = {}", min);
    println!("max_us       = {}", max);
    println!();
    println!("--- side-by-side vs Python probe (bench-e2e-latest.json) ---");
    match baseline {
        Some((py_p50, py_p99, py_n)) => {
            let delta_p50 = py_p50 - p50 as f64;
            let pct = if p50 > 0 {
                (delta_p50 / py_p50) * 100.0
            } else {
                0.0
            };
            println!(
                "{:<14} {:>14} {:>14} {:>10}",
                "metric", "python", "rust", "delta",
            );
            println!(
                "{:<14} {:>14.0} {:>14} {:>10.0}",
                "p50_us", py_p50, p50, delta_p50,
            );
            println!(
                "{:<14} {:>14.0} {:>14} {:>10.0}",
                "p99_us",
                py_p99,
                p99,
                py_p99 - p99 as f64,
            );
            println!("{:<14} {:>14} {:>14}", "n", py_n, n);
            println!("python overhead at p50 ≈ {:.1}% of Python total", pct,);
        }
        None => {
            println!(
                "(no e2e_us block in {} — run `make perf-load` first)",
                args.baseline.display()
            );
        }
    }
}

#[cfg(test)]
mod bench_probe_test;
