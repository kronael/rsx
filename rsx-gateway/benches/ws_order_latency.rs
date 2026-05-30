//! Per-order round-trip latency against the REAL rsx-gateway.
//!
//! Drives the live gateway over its actual WebSocket + REST
//! transport with WARMED (connection-amortized) clients. This is
//! the user-facing baseline for the planned egress-tile-split
//! (the gateway today runs a single monoio reactor whose
//! casting-recv poll loop shares time with WS egress — see
//! .diary/20260530.md "GATEWAY LATENCY ROOT CAUSE").
//!
//! Three workloads (each: per-order submit->reply RTT, p50/p99/
//! p999/max):
//!   1. WS single warmed stream: one connection, 100k orders
//!      back-to-back (closed-loop, labelled). Amortizes the
//!      connection -> the pure per-order gateway latency.
//!   2. WS parallel users: 100 connections (~100 users), each
//!      fires ~1k orders, all warmed. Realistic concurrency where
//!      the single-reactor sharing shows.
//!   3. REST/TCP baseline: GET /health over a fresh TCP conn
//!      (gateway closes after each REST response) -> the
//!      connect + HTTP round-trip floor to contrast with WS.
//!
//! All order workloads submit non-crossing GTC limit BUYs at a low
//! price (empty/own-side book): each order RESTS and produces
//! EXACTLY ONE reply ({"U":[oid,1,...]} status RESTING), giving a
//! clean 1-order:1-reply mapping (no fill/done multi-frame
//! ambiguity, no market maker needed). On a single stream with one
//! in-flight order, the next non-heartbeat frame IS that order's
//! reply (FIFO) — no cid/oid correlation needed. The MEASURED
//! submit->reply RTT covers the full GW->risk->ME->GW round trip
//! (decode, margin check, WAL accept, book insert, OrderInserted
//! emit, return path). Per the oracle design review, the
//! deterministic single outcome is what makes the per-order RTT
//! honest. To measure the MATCHING path instead, run a maker + cross
//! the book (the harness counts fills separately).
//!
//! SLAB BUDGET: resting orders are never cancelled, so they pile up
//! in rsx-book's fixed 65_536-deep order slab (ME panics beyond it).
//! The harness REFUSES to run if single_n + warmup + the parallel
//! total would exceed it on one ME process. Run against a FRESH ME
//! (empty book). IOC (tif=1) would avoid resting entirely, but
//! IOC-with-no-liquidity currently RESTS instead of cancelling
//! end-to-end (logged in bugs.md), so GTC-resting is used.
//!
//! Client is hand-rolled WS framing over std::net::TcpStream (no
//! async runtime, no WS library) to keep client-side overhead
//! minimal and to MEASURE it (a self-loopback echo calibration
//! reports the client's own send->recv cost so the gateway
//! number is honest).
//!
//! Not a criterion bench (harness = false): it needs a live
//! gateway + downstream cluster, so it is run by hand, not in
//! `make perf`. Env knobs:
//!   RSX_GW_WS_ADDR     (default 127.0.0.1:8080)
//!   RSX_GW_HTTP_ADDR   (default 127.0.0.1:8080)
//!   RSX_GW_JWT_SECRET  (default dev secret)
//!   RSX_BENCH_SYMBOL   (default 10 = PENGU)
//!   RSX_BENCH_PRICE    (default 1)   tick units
//!   RSX_BENCH_QTY      (default 100000) lot units (1 lot of PENGU)
//!   RSX_BENCH_SINGLE_N (default 100000)
//!   RSX_BENCH_PAR_CONN (default 100)
//!   RSX_BENCH_PAR_N    (default 1000) per connection
//!   RSX_BENCH_WARMUP   (default 1000) discarded per stream
//!   RSX_BENCH_REST_N   (default 5000)

use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use rsx_gateway::jwt::Claims;
use std::io::Read;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const DEV_SECRET: &str = "rsx-dev-secret-not-for-prod-padpad";
/// Time-in-force: 0 == GTC (see rsx-types TimeInForce). A non-
/// crossing GTC limit rests -> exactly one RESTING reply.
const TIF_GTC: u8 = 0;

fn env_str(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn mint_jwt(secret: &str, user_id: u32) -> String {
    let claims = Claims {
        sub: format!("bench:{user_id}"),
        user_id: Some(user_id),
        exp: now_secs() + 3600,
        nbf: None,
        // Unique jti per connection — the gateway JtiTracker
        // rejects replays and missing jti round-trips fine but
        // we mint fresh so 100 parallel handshakes never collide.
        jti: Some(format!("{user_id}-{}", rand_hex())),
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("jwt encode")
}

static RAND: AtomicU32 = AtomicU32::new(0x9e3779b9);

/// Fresh per-frame WS masking key (xorshift; not crypto, but
/// RFC 6455 only requires unpredictability vs the application).
fn next_mask() -> u32 {
    let mut x = RAND.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    RAND.store(x, Ordering::Relaxed);
    x
}

/// Cheap unique-enough hex for jti / cid suffixes. Not crypto.
fn rand_hex() -> String {
    let mut x = RAND.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    RAND.store(x, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{:08x}{:08x}", x, t)
}

// ── minimal WS client (RFC 6455 client framing) ────────────

struct WsConn {
    stream: TcpStream,
    rbuf: Vec<u8>,
}

impl WsConn {
    /// Connect + HTTP upgrade with Authorization: Bearer <jwt>.
    fn connect(addr: &str, jwt: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let key = "dGhlIHNhbXBsZSBub25jZQ=="; // fixed client nonce
        let host = addr;
        let req = format!(
            "GET / HTTP/1.1\r\n\
             Host: {host}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Authorization: Bearer {jwt}\r\n\
             \r\n",
        );
        let mut conn = WsConn { stream, rbuf: Vec::with_capacity(4096) };
        conn.stream.write_all(req.as_bytes())?;
        conn.stream.flush()?;
        // Read the 101 response up to the header terminator.
        conn.read_until_headers_end()?;
        Ok(conn)
    }

    fn read_until_headers_end(&mut self) -> std::io::Result<()> {
        let mut tmp = [0u8; 1024];
        loop {
            if let Some(pos) = find_subslice(&self.rbuf, b"\r\n\r\n") {
                let status = std::str::from_utf8(&self.rbuf[..pos])
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                if !status.contains("101") {
                    return Err(std::io::Error::other(format!(
                        "ws upgrade failed: {status}"
                    )));
                }
                // Drain the header bytes; keep any frame leftover.
                self.rbuf.drain(..pos + 4);
                return Ok(());
            }
            let n = self.stream.read(&mut tmp)?;
            if n == 0 {
                return Err(std::io::Error::other("eof during handshake"));
            }
            self.rbuf.extend_from_slice(&tmp[..n]);
        }
    }

    /// Send a masked text frame (client frames MUST be masked,
    /// RFC 6455 §5.3, with a fresh per-frame masking key).
    fn send_text(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x81); // FIN + opcode text
        let m = next_mask();
        let mask = m.to_be_bytes();
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len < 65536 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        for (i, b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i & 3]);
        }
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// Read the next server text frame payload (server frames are
    /// unmasked). Skips control frames (ping/pong) transparently.
    fn read_text(&mut self) -> std::io::Result<Vec<u8>> {
        loop {
            let (opcode, payload) = self.read_frame()?;
            match opcode {
                0x1 => return Ok(payload),
                0x8 => return Err(std::io::Error::other("server close")),
                _ => continue, // ping/pong/cont — ignore
            }
        }
    }

    fn read_frame(&mut self) -> std::io::Result<(u8, Vec<u8>)> {
        self.fill_at_least(2)?;
        let b0 = self.rbuf[0];
        let b1 = self.rbuf[1];
        let opcode = b0 & 0x0f;
        let masked = b1 & 0x80 != 0;
        let mut len = (b1 & 0x7f) as usize;
        let mut hdr = 2;
        if len == 126 {
            self.fill_at_least(4)?;
            len = u16::from_be_bytes([self.rbuf[2], self.rbuf[3]]) as usize;
            hdr = 4;
        } else if len == 127 {
            self.fill_at_least(10)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(&self.rbuf[2..10]);
            len = u64::from_be_bytes(a) as usize;
            hdr = 10;
        }
        let mask_len = if masked { 4 } else { 0 };
        self.fill_at_least(hdr + mask_len + len)?;
        let mask = if masked {
            [
                self.rbuf[hdr],
                self.rbuf[hdr + 1],
                self.rbuf[hdr + 2],
                self.rbuf[hdr + 3],
            ]
        } else {
            [0; 4]
        };
        let start = hdr + mask_len;
        let mut payload = self.rbuf[start..start + len].to_vec();
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i & 3];
            }
        }
        self.rbuf.drain(..start + len);
        Ok((opcode, payload))
    }

    fn fill_at_least(&mut self, n: usize) -> std::io::Result<()> {
        let mut tmp = [0u8; 8192];
        while self.rbuf.len() < n {
            let r = self.stream.read(&mut tmp)?;
            if r == 0 {
                return Err(std::io::Error::other("eof"));
            }
            self.rbuf.extend_from_slice(&tmp[..r]);
        }
        Ok(())
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ── order frame + percentiles ──────────────────────────────

/// Compact `{N:[sym,side,px,qty,cid,tif]}` order frame.
fn order_frame(sym: u64, side: u8, px: i64, qty: i64, cid: &str, tif: u8) -> Vec<u8> {
    format!("{{\"N\":[{sym},{side},{px},{qty},\"{cid}\",{tif}]}}")
        .into_bytes()
}

fn cid20(seed: &str) -> String {
    let mut s = seed.to_string();
    s.truncate(20);
    while s.len() < 20 {
        s.push('0');
    }
    s
}

struct Stats {
    p50: u64,
    p99: u64,
    p999: u64,
    max: u64,
    min: u64,
    mean: u64,
    n: usize,
}

/// Nearest-rank-ish percentiles by `round((n-1)*p)`. (Differs
/// slightly from ceil nearest-rank at small n; fine at our n.)
fn percentiles(mut v: Vec<u64>) -> Stats {
    v.sort_unstable();
    let n = v.len();
    let pick = |p: f64| -> u64 {
        if n == 0 {
            return 0;
        }
        let idx = ((n as f64 - 1.0) * p).round() as usize;
        v[idx.min(n - 1)]
    };
    let sum: u128 = v.iter().map(|x| *x as u128).sum();
    Stats {
        p50: pick(0.50),
        p99: pick(0.99),
        p999: pick(0.999),
        max: *v.last().unwrap_or(&0),
        min: *v.first().unwrap_or(&0),
        mean: if n > 0 { (sum / n as u128) as u64 } else { 0 },
        n,
    }
}

/// Classify a server WS frame by its 1-letter type + status.
/// rsx-gateway records.rs serializes quoted keys: {"U":[...]}.
fn classify(reply: &[u8]) -> ReplyKind {
    let s = std::str::from_utf8(reply).unwrap_or("");
    if s.starts_with("{\"F\"") {
        ReplyKind::Fill
    } else if s.starts_with("{\"U\"") {
        // {"U":[oid,status,filled,remaining,reason]} — status is the
        // 2nd array element. 1 == RESTING (expected for a non-
        // crossing limit), 0 == FILLED, 2 == CANCELLED, 3 == FAILED.
        match order_update_status(s) {
            Some(1) => ReplyKind::Rested,
            Some(0) => ReplyKind::Filled,
            Some(2) => ReplyKind::Cancelled,
            Some(3) => ReplyKind::Failed,
            _ => ReplyKind::Other,
        }
    } else if s.starts_with("{\"E\"") {
        ReplyKind::Error
    } else if s.starts_with("{\"H\"") {
        ReplyKind::Heartbeat
    } else {
        ReplyKind::Other
    }
}

/// Extract status (2nd array element) from {"U":["<oid>",status,..]}.
/// The oid is a quoted hex string with no comma, so splitting on
/// ',' puts the status integer at index 1.
fn order_update_status(s: &str) -> Option<u8> {
    s.split(',').nth(1)?.trim().parse::<u8>().ok()
}

#[derive(Clone, Copy, Default)]
struct Outcome {
    rested: u64,
    filled: u64,
    cancelled: u64,
    failed: u64,
    errors: u64,
    other: u64,
    /// Orders whose send/read errored (conn dropped) — NOT in the
    /// RTT samples, surfaced so percentiles aren't read as complete.
    transport_fail: u64,
}

enum ReplyKind {
    Fill,
    Rested,
    Filled,
    Cancelled,
    Failed,
    Error,
    Heartbeat,
    Other,
}

/// Send `count` orders on a warmed connection, closed-loop
/// (submit, block for reply, record RTT). Heartbeats are skipped
/// (not counted as an order reply). Returns RTTs (us) + outcome.
///
/// PAIRING INVARIANT: exactly one order is in flight at a time on
/// this stream, and the gateway preserves FIFO per connection, so
/// the next non-heartbeat frame IS the reply to the order we just
/// sent — no cid/oid correlation is needed. This holds ONLY because
/// (a) we never pipeline (one submit -> one blocking read) and
/// (b) a non-crossing limit order produces exactly one frame
/// ({"U":[oid,1,..]} status RESTING). `out.rested == n` confirms
/// the invariant held; any filled/failed/other > 0 means the book
/// unexpectedly crossed or a reject occurred and the pairing
/// assumption (and the RTTs) should be distrusted.
///
/// BOOK SIZE: resting orders accumulate in rsx-book's order slab,
/// which is a fixed 65_536 deep (ME panics "slab exhausted" beyond
/// it). The caller MUST keep (single_n + warmup) and the parallel
/// total within that budget on a single ME process — see `main`.
#[allow(clippy::too_many_arguments)]
fn run_stream(
    conn: &mut WsConn,
    sym: u64,
    side: u8,
    px: i64,
    qty: i64,
    tif: u8,
    cid_prefix: &str,
    warmup: u64,
    count: u64,
) -> (Vec<u64>, Outcome, Duration) {
    let mut rtts = Vec::with_capacity(count as usize);
    let mut out = Outcome::default();
    let total = warmup + count;
    // Wall clock of the MEASURED window only (warmup excluded), so
    // achieved-rate reflects measured throughput, not connect/warmup.
    let mut measured_start: Option<Instant> = None;
    for i in 0..total {
        if i == warmup {
            measured_start = Some(Instant::now());
        }
        let cid = cid20(&format!("{cid_prefix}{i}"));
        let frame = order_frame(sym, side, px, qty, &cid, tif);
        let t0 = Instant::now();
        if conn.send_text(&frame).is_err() {
            if i >= warmup {
                out.transport_fail += total - i;
            }
            break;
        }
        // Block until we get a real order reply (skip heartbeats).
        let reply = loop {
            match conn.read_text() {
                Ok(r) => match classify(&r) {
                    ReplyKind::Heartbeat => continue,
                    _ => break r,
                },
                Err(_) => {
                    if i >= warmup {
                        out.transport_fail += total - i;
                    }
                    let dur = measured_start.map(|s| s.elapsed()).unwrap_or_default();
                    return (rtts, out, dur);
                }
            }
        };
        let dt = t0.elapsed().as_micros() as u64;
        if i >= warmup {
            rtts.push(dt);
            match classify(&reply) {
                ReplyKind::Rested => out.rested += 1,
                ReplyKind::Fill | ReplyKind::Filled => out.filled += 1,
                ReplyKind::Cancelled => out.cancelled += 1,
                ReplyKind::Failed => out.failed += 1,
                ReplyKind::Error => out.errors += 1,
                _ => out.other += 1,
            }
        }
    }
    let dur = measured_start.map(|s| s.elapsed()).unwrap_or_default();
    (rtts, out, dur)
}

fn print_table(label: &str, s: &Stats, out: &Outcome, achieved_rate: f64, mode: &str) {
    println!("\n=== {label} ===");
    println!("  mode         : {mode}");
    println!("  orders       : {}", s.n);
    println!("  achieved rate: {achieved_rate:.0} orders/sec");
    println!(
        "  outcomes     : rested={} filled={} cancelled={} failed={} errors={} other={} transport_fail={}",
        out.rested, out.filled, out.cancelled, out.failed, out.errors, out.other, out.transport_fail
    );
    // Clean iff every measured order produced its expected single
    // terminal reply (resting -> RESTING) with no transport failures.
    let clean = out.rested == s.n as u64 && out.transport_fail == 0;
    println!(
        "  pairing      : {}",
        if clean {
            "OK (every measured order -> 1 RESTING reply, FIFO one-in-flight)"
        } else {
            "SUSPECT (unexpected outcome or transport failures -> distrust RTTs)"
        }
    );
    println!("  RTT us  min={} mean={}", s.min, s.mean);
    println!("          p50={}  p99={}  p999={}  max={}", s.p50, s.p99, s.p999, s.max);
}

// ── REST baseline ──────────────────────────────────────────

/// One REST GET /health over a fresh TCP connection (gateway
/// sends Connection: close, so each request = connect + request +
/// response + close). Returns RTT us, or None on failure.
fn rest_once(addr: &str) -> Option<u64> {
    let t0 = Instant::now();
    let mut stream = TcpStream::connect(addr).ok()?;
    stream.set_nodelay(true).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let req = format!("GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    stream.flush().ok()?;
    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 1024];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if find_subslice(&buf, b"{\"status\":\"ok\"}").is_some() {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    if find_subslice(&buf, b"200 OK").is_some() {
        Some(t0.elapsed().as_micros() as u64)
    } else {
        None
    }
}

// ── client-overhead calibration (loopback echo) ────────────

/// Stand up a localhost TCP echo, run the same masked-frame
/// send + unmasked-frame read against it, and report the
/// client's own send->recv cost. This is the floor we are
/// adding on top of the gateway; the gateway numbers above
/// include it.
fn calibrate_client_overhead(iters: u64) -> Stats {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            s.set_nodelay(true).ok();
            let mut rbuf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 8192];
            // Echo each client text frame back as an unmasked
            // server text frame, mirroring the gateway direction.
            loop {
                // parse one masked client frame from rbuf
                while !try_echo_one(&mut s, &mut rbuf) {
                    match s.read(&mut tmp) {
                        Ok(0) => return,
                        Ok(n) => rbuf.extend_from_slice(&tmp[..n]),
                        Err(_) => return,
                    }
                }
            }
        }
    });
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut conn = WsConn { stream, rbuf: Vec::new() };
    let payload = b"{\"N\":[10,0,1,100000,\"calib00000000000000\",0]}";
    let mut rtts = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let t0 = Instant::now();
        conn.send_text(payload).unwrap();
        let _ = conn.read_text().unwrap();
        rtts.push(t0.elapsed().as_micros() as u64);
    }
    drop(conn);
    let _ = handle.join();
    percentiles(rtts)
}

/// Parse one masked client frame from `rbuf` and echo its payload
/// back as an unmasked server text frame. Returns true if a frame
/// was consumed, false if more bytes are needed.
fn try_echo_one(s: &mut TcpStream, rbuf: &mut Vec<u8>) -> bool {
    if rbuf.len() < 2 {
        return false;
    }
    let b1 = rbuf[1];
    let masked = b1 & 0x80 != 0;
    let mut len = (b1 & 0x7f) as usize;
    let mut hdr = 2;
    if len == 126 {
        if rbuf.len() < 4 {
            return false;
        }
        len = u16::from_be_bytes([rbuf[2], rbuf[3]]) as usize;
        hdr = 4;
    } else if len == 127 {
        if rbuf.len() < 10 {
            return false;
        }
        let mut a = [0u8; 8];
        a.copy_from_slice(&rbuf[2..10]);
        len = u64::from_be_bytes(a) as usize;
        hdr = 10;
    }
    let mask_len = if masked { 4 } else { 0 };
    if rbuf.len() < hdr + mask_len + len {
        return false;
    }
    let mask = if masked {
        [rbuf[hdr], rbuf[hdr + 1], rbuf[hdr + 2], rbuf[hdr + 3]]
    } else {
        [0u8; 4]
    };
    let start = hdr + mask_len;
    let mut payload = rbuf[start..start + len].to_vec();
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }
    rbuf.drain(..start + len);
    // unmasked server text frame
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.push(0x81);
    let l = payload.len();
    if l < 126 {
        frame.push(l as u8);
    } else if l < 65536 {
        frame.push(126);
        frame.extend_from_slice(&(l as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(l as u64).to_be_bytes());
    }
    frame.extend_from_slice(&payload);
    s.write_all(&frame).ok();
    s.flush().ok();
    true
}

fn main() {
    let ws_addr = env_str("RSX_GW_WS_ADDR", "127.0.0.1:8080");
    let http_addr = env_str("RSX_GW_HTTP_ADDR", "127.0.0.1:8080");
    let secret = env_str("RSX_GW_JWT_SECRET", DEV_SECRET);
    let sym = env_u64("RSX_BENCH_SYMBOL", 10);
    let px = env_u64("RSX_BENCH_PRICE", 1) as i64;
    let qty = env_u64("RSX_BENCH_QTY", 100_000) as i64;
    // Defaults sized to fit rsx-book's 65_536-deep order slab on one
    // ME (single 31k + parallel 30k = 61k < 65k). 100k on one stream
    // would exhaust the slab; raise these only against a maker/IOC
    // setup that doesn't leave orders resting.
    let single_n = env_u64("RSX_BENCH_SINGLE_N", 30_000);
    let par_conn = env_u64("RSX_BENCH_PAR_CONN", 100);
    let par_n = env_u64("RSX_BENCH_PAR_N", 250);
    let warmup = env_u64("RSX_BENCH_WARMUP", 1000);
    let par_warmup = env_u64("RSX_BENCH_PAR_WARMUP", 50);
    let rest_n = env_u64("RSX_BENCH_REST_N", 5000);

    // Slab budget guard: every resting order stays in the book.
    const SLAB_CAP: u64 = 65_536;
    let total_resting = single_n + warmup + par_conn * (par_n + par_warmup);
    if total_resting >= SLAB_CAP {
        eprintln!(
            "REFUSING: resting orders {total_resting} >= rsx-book slab {SLAB_CAP}.\n\
             single_n+warmup ({}) + par_conn*(par_n+par_warmup) ({}) would panic the ME.\n\
             Lower the counts, or use a maker/IOC setup so orders don't rest.",
            single_n + warmup,
            par_conn * (par_n + par_warmup),
        );
        std::process::exit(2);
    }

    println!("rsx-gateway WS order-latency bench (real gateway, warmed clients)");
    println!(
        "  ws={ws_addr} http={http_addr} sym={sym} px={px} qty={qty}\n  \
         single_n={single_n} par_conn={par_conn} par_n={par_n} warmup={warmup} \
         par_warmup={par_warmup} rest_n={rest_n} (resting total {total_resting} < slab {SLAB_CAP})"
    );

    // 0. client-overhead calibration (loopback echo).
    let calib = calibrate_client_overhead(20_000);
    println!("\n=== CLIENT OVERHEAD (loopback echo, no gateway) ===");
    println!(
        "  send->recv us  min={} p50={} p99={} max={}",
        calib.min, calib.p50, calib.p99, calib.max
    );
    println!(
        "  (client framing + loopback TCP + echo-thread floor; included in the\n   \
         gateway RTTs and APPROXIMATELY subtractable, not a rigorous correction)"
    );

    // 1. WS single warmed stream (closed-loop, one connection).
    {
        let jwt = mint_jwt(&secret, 1);
        match WsConn::connect(&ws_addr, &jwt) {
            Ok(mut conn) => {
                let (rtts, out, dur) = run_stream(
                    &mut conn, sym, 0, px, qty, TIF_GTC,"s1-", warmup, single_n,
                );
                let rate = rtts.len() as f64 / dur.as_secs_f64().max(1e-9);
                let s = percentiles(rtts);
                print_table(
                    &format!("WS SINGLE WARMED STREAM (1 conn, {single_n} orders)"),
                    &s,
                    &out,
                    rate,
                    "closed-loop (one in-flight order at a time on a single stream)",
                );
            }
            Err(e) => println!("\nWS single connect failed: {e}"),
        }
    }

    // 2. WS parallel users (par_conn connections, par_n each, warmed).
    // A Barrier makes every connection enter the MEASURED window
    // together (after its own connect + warmup), so the workload is
    // genuinely concurrent and the aggregate rate is measured over a
    // shared window, not polluted by staggered connect/warmup.
    {
        let ws_addr = Arc::new(ws_addr.clone());
        let secret = Arc::new(secret.clone());
        let barrier = Arc::new(std::sync::Barrier::new(par_conn as usize));
        let mut handles = Vec::with_capacity(par_conn as usize);
        for c in 0..par_conn {
            let ws_addr = ws_addr.clone();
            let secret = secret.clone();
            let barrier = barrier.clone();
            handles.push(thread::spawn(move || {
                // Distinct user per connection (~par_conn users).
                let uid = (c as u32) + 1;
                let jwt = mint_jwt(&secret, uid);
                let mut conn = match WsConn::connect(&ws_addr, &jwt) {
                    Ok(c) => c,
                    Err(_) => return (Vec::new(), Outcome::default(), Duration::ZERO),
                };
                // Per-connection warmup BEFORE the barrier.
                let _ = run_stream(&mut conn, sym, 0, px, qty, TIF_GTC, &format!("w{c}-"), 0, par_warmup);
                barrier.wait();
                let start = Instant::now();
                let (rtts, out, _) =
                    run_stream(&mut conn, sym, 0, px, qty, TIF_GTC,&format!("p{c}-"), 0, par_n);
                (rtts, out, start.elapsed())
            }));
        }
        let mut all = Vec::new();
        let mut out = Outcome::default();
        let mut window = Duration::ZERO;
        for h in handles {
            if let Ok((rtts, o, dur)) = h.join() {
                all.extend(rtts);
                out.rested += o.rested;
                out.filled += o.filled;
                out.cancelled += o.cancelled;
                out.failed += o.failed;
                out.errors += o.errors;
                out.other += o.other;
                out.transport_fail += o.transport_fail;
                window = window.max(dur);
            }
        }
        // Aggregate measured throughput over the shared window.
        let rate = all.len() as f64 / window.as_secs_f64().max(1e-9);
        let s = percentiles(all);
        print_table(
            &format!("WS PARALLEL USERS ({par_conn} conns x {par_n} orders, warmed)"),
            &s,
            &out,
            rate,
            "concurrent closed-loop streams, barrier-synced (realistic concurrency; single-reactor sharing)",
        );
    }

    // 3. REST/TCP baseline (fresh conn per request).
    {
        // warmup
        for _ in 0..warmup.min(200) {
            let _ = rest_once(&http_addr);
        }
        let t0 = Instant::now();
        let mut rtts = Vec::with_capacity(rest_n as usize);
        let mut fail = 0u64;
        for _ in 0..rest_n {
            match rest_once(&http_addr) {
                Some(us) => rtts.push(us),
                None => fail += 1,
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let rate = rtts.len() as f64 / secs.max(1e-9);
        let s = percentiles(rtts);
        println!("\n=== REST/TCP BASELINE (GET /health, fresh TCP conn each) ===");
        println!("  mode         : connect + HTTP/1.1 request + response + close");
        println!("  requests     : {} (failures={})", s.n, fail);
        println!("  achieved rate: {rate:.0} req/sec");
        println!("  RTT us  min={} mean={}", s.min, s.mean);
        println!(
            "          p50={}  p99={}  p999={}  max={}",
            s.p50, s.p99, s.p999, s.max
        );
    }

    println!("\nNOTE: This measures the gateway AS-IS (single monoio reactor,");
    println!("poll-loop casting egress). It is the BASELINE for the planned");
    println!("egress-tile-split (pinned busy-spin casting-recv -> SPSC to WS).");
    println!("CAVEATS: closed-loop (one in-flight order/stream) -> NOT a");
    println!("saturation/coordinated-omission test; the CLIENT OVERHEAD floor");
    println!("above is included and ~subtractable; read_timeout is a failure");
    println!("bound, not a latency clip; warmup leaves resting orders in the");
    println!("book (realistic, but book depth grows with warmup*conns); run");
    println!("clients off the gateway/risk/ME cores (pinned 1-4) to avoid noise.");
}
