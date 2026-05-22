//! bench-probe — native Rust E2E latency probe.
//!
//! Connects to the gateway WebSocket directly with
//! tokio-tungstenite, mints a JWT inline, sends N order
//! frames, waits for the matching `F` (fill) frame whose
//! `taker_oid` == our oid, and records `perf_counter`-style
//! deltas. Prints p50/p95/p99 and a side-by-side comparison
//! with the Python `make latency-publish` numbers held in
//! `bench-baseline.json`.
//!
//! Goal: isolate the Python aiohttp overhead in the existing
//! `/api/latency-probe` flow. The Python probe holds ~12 ms
//! p50; we expect this native client to be substantially
//! lower if Python is the floor, or roughly equal if the
//! cost lives downstream (risk/ME/PG).
//!
//! Usage:
//!   bench-probe \
//!     --gateway ws://127.0.0.1:8080 \
//!     --playground http://127.0.0.1:49171 \
//!     --symbol-id 10 \
//!     --jwt-secret $RSX_GW_JWT_SECRET \
//!     --n 2000

use clap::Parser;
use futures_util::SinkExt;
use futures_util::StreamExt;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use jsonwebtoken::encode;
use serde::Deserialize;
use serde::Serialize;
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
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;

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
    /// Lot size for the order qty (raw i64). Defaults to
    /// 100_000 which matches the default symbol_id=10 config.
    #[arg(long, default_value_t = 100_000)]
    lot_size: i64,
    /// Path to bench-baseline.json for side-by-side compare.
    #[arg(long, default_value = "bench-baseline.json")]
    baseline: PathBuf,
}

#[derive(Serialize)]
struct Claims<'a> {
    sub: String,
    user_id: u32,
    aud: &'a str,
    iss: &'a str,
    exp: u64,
    /// JWT ID — required by the gateway since the JtiTracker
    /// wire-through (`rsx-gateway/src/ws.rs::extract_user_and_record_jti`).
    /// A token without `jti` is rejected with `missing jti`.
    jti: String,
}

#[derive(Deserialize, Debug)]
struct BookSnap {
    asks: Vec<BookLevel>,
}

#[derive(Deserialize, Debug)]
struct BookLevel {
    px: i64,
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

fn mint_jwt(secret: &str, user_id: u32) -> String {
    // Each probe call gets a fresh `jti`. Reusing a jti
    // across handshakes would trip the JtiTracker on the
    // second connection. Use perf_counter_ns to avoid
    // sub-second collisions (warmup + measurement both
    // run multiple probes per second).
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let jti = format!("bench-{}-{}", user_id, nonce);
    let claims = Claims {
        sub: format!("bench-probe:{}", user_id),
        user_id,
        aud: "rsx-gateway",
        iss: "rsx-auth",
        exp: unix_now_secs() + 300,
        jti,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap_or_else(|e| die(format!("jwt encode failed: {}", e)))
}

/// Minimal blocking HTTP GET that returns the body bytes.
/// Only supports http:// (loopback). Used to read /api/book.
async fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let stripped = url.strip_prefix("http://")
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

async fn fetch_best_ask(
    playground: &str,
    symbol_id: u32,
) -> Result<i64, String> {
    let url = format!(
        "{}/api/book/{}",
        playground.trim_end_matches('/'),
        symbol_id,
    );
    let body = http_get(&url).await?;
    let snap: BookSnap = serde_json::from_slice(&body)
        .map_err(|e| format!("parse book: {} body={}",
            e, String::from_utf8_lossy(&body)))?;
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
    lot_size: i64,
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
    headers.insert(
        "Sec-WebSocket-Version",
        "13".parse().unwrap(),
    );
    headers.insert("Connection", "Upgrade".parse().unwrap());
    headers.insert("Upgrade", "websocket".parse().unwrap());

    // Resolve host:port from the URL.
    let url_no_scheme = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .ok_or_else(|| format!("bad ws url: {}", ws_url))?;
    let host_part = url_no_scheme
        .split('/')
        .next()
        .unwrap_or(url_no_scheme);
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
        "N": [symbol_id, 0, cross_px, lot_size, cid, 0],
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

    let timeout = Duration::from_secs_f64(args.timeout_s);
    // Mint a fresh JWT (with a fresh jti) per handshake. The
    // gateway's JtiTracker rejects replayed jtis, so reusing a
    // single token across N probes would fail every call but
    // the first.

    // Warmup
    for i in 0..args.warmup {
        let cid = format!("warm-{}", i);
        let token = mint_jwt(&secret, args.user_id);
        if let Err(e) = one_probe(
            &args.gateway,
            &token,
            args.symbol_id,
            cross_px,
            args.lot_size,
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
        let cid = format!("bp-{}-{}", i, unix_now_secs() % 1_000_000);
        let token = mint_jwt(&secret, args.user_id);
        match one_probe(
            &args.gateway,
            &token,
            args.symbol_id,
            cross_px,
            args.lot_size,
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
    println!("--- side-by-side vs Python probe (bench-baseline.json e2e_us) ---");
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
                "p99_us", py_p99, p99, py_p99 - p99 as f64,
            );
            println!("{:<14} {:>14} {:>14}", "n", py_n, n);
            println!(
                "python overhead at p50 ≈ {:.1}% of Python total",
                pct,
            );
        }
        None => {
            println!("(no e2e_us block in {} — run `make latency-publish` first)",
                args.baseline.display());
        }
    }
}
