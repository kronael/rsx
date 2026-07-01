//! Reusable health + load-metrics HTTP endpoint for RSX daemons.
//!
//! # Design
//!
//! The health server runs on a dedicated `std::thread` — never on the
//! busy-spin loop, monoio reactor, or tokio runtime of the daemon it
//! monitors. The hot path is zero-cost: it does **relaxed atomic stores**
//! on an `Arc<LoadGauges>` struct. The health thread reads those atomics
//! with `Ordering::Relaxed` whenever a request arrives (off the hot path).
//!
//! # Hot-path contract
//!
//! The daemon allocates one `Arc<LoadGauges>` at startup and clones the
//! `Arc` into the health server. From the hot loop it calls:
//!
//! ```text
//! gauges.orders_processed.fetch_add(1, Ordering::Relaxed);
//! gauges.resp_ring_used.store(n, Ordering::Relaxed);
//! gauges.live.store(true, Ordering::Relaxed);
//! ```
//!
//! No mutex, no heap allocation per message — just a single 64-bit or
//! bool atomic store per counter that the daemon already tracks.
//!
//! # k8s usage
//!
//! - `GET /health`  → 200/503 — liveness probe (restart pod when 503)
//! - `GET /ready`   → 200/503 — readiness probe (remove from Service → shed)
//! - `GET /metrics` → 200 + JSON snapshot — HPA or manual load inspection

use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::warn;

/// Daemon lifecycle state, stored in `LoadGauges::state_idx`
/// as `state as u64`. `state_label` decodes it back to a
/// human string. Values are stable wire-visible integers
/// (health `/metrics` state label depends on them).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// Pre-init / unknown (zero-init default).
    Boot = 0,
    /// Applying the replication stream, not yet promoted.
    WarmCatchup = 1,
    /// Sole lock holder, serving traffic.
    Live = 2,
    /// Fault detected; recovering via replay.
    Faulted = 3,
    /// Generic "running" for daemons without a warm/live split.
    Running = 4,
}

/// One queue gauge: name, used slots, capacity.
pub struct QueueGauge {
    pub name: &'static str,
    pub used: u64,
    pub cap: u64,
}

/// One named counter.
pub struct CounterGauge {
    pub name: &'static str,
    pub value: u64,
}

/// A point-in-time snapshot of daemon health and load.
/// All fields are cheaply readable from atomics.
pub struct HealthSnapshot {
    /// Process is alive and not in a fatal error state.
    pub live: bool,
    /// Process is ready to serve traffic (not warming up,
    /// not overloaded). k8s removes the pod from the
    /// Service load-balancer when this is false.
    pub ready: bool,
    /// Fraction [0.0, 1.0]: highest queue occupancy across
    /// all monitored rings. Used by HPA to scale out.
    pub saturation: f64,
    /// Per-ring occupancy snapshot.
    pub queues: Vec<QueueGauge>,
    /// Named event counters (orders, fills, drops, etc.).
    pub counters: Vec<CounterGauge>,
    /// Human-readable daemon state label (e.g. "live",
    /// "warm_catchup", "faulted").
    pub state: &'static str,
}

impl HealthSnapshot {
    /// Render the snapshot as a hand-rolled JSON string.
    /// No serde needed — the structure is flat and fixed.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push('{');
        out.push_str(&format!(
            "\"live\":{},\"ready\":{},\
             \"saturation\":{:.4},\"state\":\"{}\"",
            self.live,
            self.ready,
            self.saturation,
            self.state,
        ));

        out.push_str(",\"queues\":[");
        for (i, q) in self.queues.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"name\":\"{}\",\"used\":{},\"cap\":{}}}",
                q.name, q.used, q.cap,
            ));
        }
        out.push(']');

        out.push_str(",\"counters\":[");
        for (i, c) in self.counters.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"name\":\"{}\",\"value\":{}}}",
                c.name, c.value,
            ));
        }
        out.push(']');
        out.push('}');
        out
    }
}

/// Atomics a daemon updates on the hot path (relaxed stores).
/// The health thread reads them with Relaxed loads — no locks,
/// no allocations, no syscalls on the hot path.
///
/// Not all fields are used by every daemon. Unused fields stay
/// at their zero-init values.
pub struct LoadGauges {
    // Liveness / readiness
    pub live: AtomicBool,
    pub ready: AtomicBool,

    // Ring occupancy (used / capacity pairs).
    // Capacity is set once at startup (non-atomic read is fine
    // since the health thread only starts after the daemon has
    // called `set_caps`).
    pub resp_ring_used: AtomicU64,
    pub resp_ring_cap: AtomicU64,
    pub accept_ring_used: AtomicU64,
    pub accept_ring_cap: AtomicU64,
    pub persist_ring_used: AtomicU64,
    pub persist_ring_cap: AtomicU64,

    // Throughput / event counters
    pub orders_processed: AtomicU64,
    pub fills_processed: AtomicU64,
    pub rejects: AtomicU64,
    pub drops: AtomicU64,
    pub publishes: AtomicU64,

    // Misc state
    pub connections: AtomicU64,
    pub pending_orders: AtomicU64,
    pub dedup_map_size: AtomicU64,
    pub lag_records: AtomicI64,

    // State label index: 0=unknown, 1=warm_catchup, 2=live,
    // 3=faulted, 4=running. Stored as u64 for atomic ops.
    pub state_idx: AtomicU64,
}

impl LoadGauges {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            live: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            resp_ring_used: AtomicU64::new(0),
            resp_ring_cap: AtomicU64::new(0),
            accept_ring_used: AtomicU64::new(0),
            accept_ring_cap: AtomicU64::new(0),
            persist_ring_used: AtomicU64::new(0),
            persist_ring_cap: AtomicU64::new(0),
            orders_processed: AtomicU64::new(0),
            fills_processed: AtomicU64::new(0),
            rejects: AtomicU64::new(0),
            drops: AtomicU64::new(0),
            publishes: AtomicU64::new(0),
            connections: AtomicU64::new(0),
            pending_orders: AtomicU64::new(0),
            dedup_map_size: AtomicU64::new(0),
            lag_records: AtomicI64::new(0),
            state_idx: AtomicU64::new(0),
        })
    }

    /// Store the daemon state (relaxed). Typed alternative to
    /// `state_idx.store(n, ...)` with a magic number.
    pub fn set_state(&self, s: DaemonState) {
        self.state_idx.store(s as u64, Ordering::Relaxed);
    }

    /// Decode state_idx → human label.
    pub fn state_label(&self) -> &'static str {
        match self.state_idx.load(Ordering::Relaxed) {
            x if x == DaemonState::WarmCatchup as u64 => "warm_catchup",
            x if x == DaemonState::Live as u64 => "live",
            x if x == DaemonState::Faulted as u64 => "faulted",
            x if x == DaemonState::Running as u64 => "running",
            _ => "unknown",
        }
    }
}

impl Default for LoadGauges {
    fn default() -> Self {
        // Satisfy trait; prefer LoadGauges::new() for Arc<Self>.
        Self {
            live: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            resp_ring_used: AtomicU64::new(0),
            resp_ring_cap: AtomicU64::new(0),
            accept_ring_used: AtomicU64::new(0),
            accept_ring_cap: AtomicU64::new(0),
            persist_ring_used: AtomicU64::new(0),
            persist_ring_cap: AtomicU64::new(0),
            orders_processed: AtomicU64::new(0),
            fills_processed: AtomicU64::new(0),
            rejects: AtomicU64::new(0),
            drops: AtomicU64::new(0),
            publishes: AtomicU64::new(0),
            connections: AtomicU64::new(0),
            pending_orders: AtomicU64::new(0),
            dedup_map_size: AtomicU64::new(0),
            lag_records: AtomicI64::new(0),
            state_idx: AtomicU64::new(0),
        }
    }
}

/// Spawn a health HTTP server on `addr` in a dedicated
/// `std::thread`. The server is OFF the hot path — it only
/// does `accept` + `read` + atomic loads + `write`.
///
/// The `snapshot` closure is called once per request on the
/// health thread; it should read from an `Arc<LoadGauges>`
/// that the daemon updates on its hot path with relaxed stores.
///
/// If the bind fails the error is logged as a warning and the
/// function returns — the daemon continues without health
/// endpoints (the env var is optional).
pub fn spawn_health_server<F>(addr: SocketAddr, snapshot: F)
where
    F: Fn() -> HealthSnapshot + Send + 'static,
{
    std::thread::Builder::new()
        .name(format!("health@{}", addr))
        .spawn(move || {
            let listener = match TcpListener::bind(addr) {
                Ok(l) => l,
                Err(e) => {
                    warn!("health: bind {} failed: {e}", addr);
                    return;
                }
            };
            tracing::info!(
                "health: listening on {}",
                addr,
            );
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("health: accept error: {e}");
                        continue;
                    }
                };
                serve_one(stream, &snapshot);
            }
        })
        .expect("health thread spawn failed");
}

fn serve_one<F>(
    mut stream: std::net::TcpStream,
    snapshot: &F,
) where
    F: Fn() -> HealthSnapshot,
{
    let mut buf = [0u8; 256];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let req = match std::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(_) => return,
    };
    let path = parse_path(req).unwrap_or("");
    let path = path.split('?').next().unwrap_or(path);

    let response = match path {
        "/health" => {
            let snap = snapshot();
            if snap.live {
                http_200(r#"{"status":"ok"}"#)
            } else {
                http_503(r#"{"status":"not_live"}"#)
            }
        }
        "/ready" => {
            let snap = snapshot();
            if snap.ready {
                http_200(r#"{"status":"ready"}"#)
            } else {
                http_503(r#"{"status":"not_ready"}"#)
            }
        }
        "/metrics" | "/loadz" => {
            let snap = snapshot();
            let body = snap.to_json();
            http_200(&body)
        }
        _ => http_404(),
    };

    if let Err(e) = stream.write_all(&response) {
        warn!("health: write error: {e}");
    }
}

fn parse_path(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.splitn(3, ' ');
    parts.next(); // method
    parts.next()  // path
}

fn http_200(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body,
    )
    .into_bytes()
}

fn http_503(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 503 Service Unavailable\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body,
    )
    .into_bytes()
}

fn http_404() -> Vec<u8> {
    let body = r#"{"error":"not found"}"#;
    format!(
        "HTTP/1.1 404 Not Found\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body,
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::io::Write;
    use std::net::TcpStream;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    fn make_snapshot(
        live: bool,
        ready: bool,
        saturation: f64,
    ) -> HealthSnapshot {
        HealthSnapshot {
            live,
            ready,
            saturation,
            queues: vec![
                QueueGauge { name: "resp", used: 10, cap: 100 },
            ],
            counters: vec![
                CounterGauge { name: "orders", value: 42 },
            ],
            state: "live",
        }
    }

    #[test]
    fn test_json_live_ready() {
        let snap = make_snapshot(true, true, 0.1);
        let j = snap.to_json();
        assert!(j.contains("\"live\":true"));
        assert!(j.contains("\"ready\":true"));
        assert!(j.contains("\"saturation\":0.1000"));
        assert!(j.contains("\"state\":\"live\""));
        assert!(j.contains("\"name\":\"resp\""));
        assert!(j.contains("\"used\":10"));
        assert!(j.contains("\"cap\":100"));
        assert!(j.contains("\"name\":\"orders\""));
        assert!(j.contains("\"value\":42"));
    }

    #[test]
    fn test_json_not_live() {
        let snap = make_snapshot(false, false, 0.0);
        let j = snap.to_json();
        assert!(j.contains("\"live\":false"));
        assert!(j.contains("\"ready\":false"));
    }

    #[test]
    fn test_http_200_503_mapping() {
        // 200 when live
        let r = http_200("ok");
        assert!(r.starts_with(b"HTTP/1.1 200"));
        // 503 when not live
        let r = http_503("down");
        assert!(r.starts_with(b"HTTP/1.1 503"));
        // 404 for unknown
        let r = http_404();
        assert!(r.starts_with(b"HTTP/1.1 404"));
    }

    #[test]
    fn test_load_gauges_relaxed() {
        let g = LoadGauges::new();
        g.orders_processed.fetch_add(5, Ordering::Relaxed);
        g.live.store(true, Ordering::Relaxed);
        g.state_idx.store(2, Ordering::Relaxed);
        assert_eq!(
            g.orders_processed.load(Ordering::Relaxed),
            5
        );
        assert!(g.live.load(Ordering::Relaxed));
        assert_eq!(g.state_label(), "live");
    }

    #[test]
    fn test_server_responds() {
        // Bind to a dynamic port, send requests via TcpStream.
        let addr: SocketAddr =
            "127.0.0.1:0".parse().expect("parse addr");
        let listener = TcpListener::bind(addr)
            .expect("bind test listener");
        let bound = listener.local_addr()
            .expect("local_addr");

        let gauges = LoadGauges::new();
        gauges.live.store(true, Ordering::Relaxed);
        gauges.ready.store(true, Ordering::Relaxed);

        let g2 = gauges.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let g = g2.clone();
                serve_one(stream, &move || {
                    let live = g.live.load(Ordering::Relaxed);
                    let ready = g.ready.load(Ordering::Relaxed);
                    HealthSnapshot {
                        live,
                        ready,
                        saturation: 0.0,
                        queues: vec![],
                        counters: vec![],
                        state: "live",
                    }
                });
            }
        });

        // Give the thread time to start
        std::thread::sleep(Duration::from_millis(20));

        // /health → 200
        let resp = do_get(bound, "GET /health HTTP/1.0\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 200"), "got: {resp}");

        // /ready → 200
        let resp = do_get(bound, "GET /ready HTTP/1.0\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 200"), "got: {resp}");

        // /metrics → 200 + JSON
        let resp = do_get(bound, "GET /metrics HTTP/1.0\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 200"), "got: {resp}");
        assert!(resp.contains("\"live\":true"), "got: {resp}");

        // /unknown → 404
        let resp = do_get(bound, "GET /unknown HTTP/1.0\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 404"), "got: {resp}");

        // Now flip live=false → /health 503
        gauges.live.store(false, Ordering::Relaxed);
        let resp = do_get(bound, "GET /health HTTP/1.0\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 503"), "got: {resp}");
    }

    fn do_get(addr: SocketAddr, req: &str) -> String {
        let mut s = TcpStream::connect(addr)
            .expect("connect to test health server");
        s.write_all(req.as_bytes()).expect("write request");
        s.shutdown(std::net::Shutdown::Write)
            .expect("shutdown write");
        let mut buf = String::new();
        s.read_to_string(&mut buf).expect("read response");
        buf
    }
}
