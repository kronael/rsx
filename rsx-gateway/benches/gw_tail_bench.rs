//! Tail-latency (p50/p99/p99.9/max) harness for the gateway: the order path
//! from a WebSocket frame to the casting datagram at risk, and the route path
//! from a casting record at the gateway to the frame on the client's socket.
//!
//! NOT Criterion: a plain `fn main` wired as a `[[bench]] harness = false`
//! target, so one run yields every percentile of the same samples:
//!   `cargo bench -p rsx-gateway --bench gw_tail_bench`
//!
//! ## What runs
//!
//! The production pieces, with only the peers faked at the casting boundary
//! (the shape of `tests/common/mod.rs`): `handle_connection` on a monoio
//! loopback listener (HTTP upgrade, JWT, frame read, parse, `admit_order`,
//! `OrderRequest` build, `CastSender::send_raw`, pending and client-id
//! insert), the `GatewayState`, and for the route path the real
//! `cast_recv::spawn` thread (`CastReceiver` busy-spin, SPSC ring, doorbell)
//! plus the reactor's drain-and-route step that `src/main.rs` runs. The
//! client and the risk stand-in are tasks on the gateway's own reactor
//! thread, so a sample includes the client's socket calls and the wakeups
//! between tasks, not the gateway's work alone.
//!
//! ## Order path (`order_accept`)
//!
//! One sample: the client writes a masked `N` text frame (symbol 0, GTC,
//! unique client order id) and awaits the `OrderRequest` datagram on the
//! risk socket. The clock runs from before the write to after the receive.
//! Between samples, outside the clock, the order is removed from the pending
//! set and the client-id map is cleared every `CLEAN_EVERY` orders, so the
//! pending set and the map stay at their warmed size instead of growing for
//! the run.
//!
//! ## Route path (`route_inserted`, `route_fill`, `route_done`)
//!
//! One sample: risk's `CastSender` sends the record to the gateway's
//! `CastReceiver`; the clock runs from before the send to after the client has
//! read the whole `U` (inserted, done) or `F` frame. The path covers the
//! recv thread, the ring, the doorbell wake, the route function (serialize,
//! `push_to_user`, egress wake), the session's write, and the client's read.
//! The record's user has a pending entry before each sample, as a live order
//! does; the fill's maker is a user with no connection here.
//!
//! The recv thread runs on its own core when the box has two or more; it
//! busy-spins, so it needs a core of its own to be measured at all. `nivcsw`
//! counts only the reactor thread. `route_events` is the drain-and-route step
//! of `src/main.rs` with its 1 ms housekeeping wake behind the doorbell.
//!
//! ## Host noise
//!
//! Each row prints the involuntary context switches of the reactor thread
//! (`nivcsw`) over the row. A p99.9 or max near a scheduler time slice with a
//! non-zero `nivcsw` is the host preempting the bench. The route rows are the
//! sensitive ones: when other runnable work shares the recv thread's core,
//! the kernel runs the spinning thread in time slices, and a record waits
//! for its next slice (a few milliseconds, one scheduler tick). A loaded box
//! therefore measures the scheduler on those rows, not the gateway.
//!
//! ## Knobs
//!
//! - `GW_TAIL_SAMPLES`: timed samples per row (default 100000); the warmup is
//!   a fifth of it.
//! - `GW_TAIL_RECV_CORE`: the recv thread's core index, or `none` to leave it
//!   unpinned.
//!
//! Warmup and the timed samples of a row each stop at a wall-clock budget
//! (3 s, 20 s); a row cut short prints the `n` it measured.
//!
//! ## Gate
//!
//! p50 and p99 of every row are written to `target/tail/rsx-gateway.json` as
//! `{name: ns}`; `scripts/bench-gate.sh` compares them with
//! `bench-baseline.json` like the Criterion estimates.
//!
//! Run on a quiet box with the RSX cluster stopped. On a shared host record
//! `nproc`, load average and `/proc/pressure/cpu` next to the numbers.

use monoio::io::AsyncReadRent;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use rsx_cast::as_bytes;
use rsx_cast::decode_payload;
use rsx_cast::CastReceiver;
use rsx_cast::CastRecord;
use rsx_cast::CastSender;
use rsx_cast::WalHeader;
use rsx_gateway::cast_recv;
use rsx_gateway::cast_recv::CastEvent;
use rsx_gateway::handler::handle_connection;
use rsx_gateway::order_id::join_order_id;
use rsx_gateway::pending::PendingOrder;
use rsx_gateway::pending::PendingStage;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_inserted;
use rsx_gateway::state::GatewayState;
use rsx_gateway::symbols::build_table;
use rsx_messages::mint_jwt;
use rsx_messages::Claims;
use rsx_messages::FillRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::OrderRequest;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::time_utils::time_ns;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use std::cell::RefCell;
use std::fmt::Write as _;
use std::future::Future;
use std::hint::black_box;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const DEFAULT_SAMPLES: usize = 100_000;
/// Wall-clock cap on a row's warmup and on its timed samples. A quiet box
/// finishes a row in seconds; on a loaded box a round trip can take
/// milliseconds, and the cap keeps the run, and `scripts/bench-gate.sh`, from
/// taking minutes. A row cut short prints the `n` it measured.
const WARMUP_BUDGET: Duration = Duration::from_secs(3);
const ROW_BUDGET: Duration = Duration::from_secs(20);
/// The reactor's fallback wake in `src/main.rs`, behind a dropped doorbell.
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_millis(1);
const CLEAN_EVERY: u64 = 1024;
const FLOOR_SAMPLES: usize = 100_000;
const RING_CAPACITY: usize = 4096;

const SECRET: &str = "gateway-tail-bench-secret-padded-to-32!";
const USER_ID: u32 = 7;
const MAKER_USER_ID: u32 = 8;
const MASK_KEY: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
const SYNC_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

struct Percentiles {
    p50: f64,
    p99: f64,
    p999: f64,
    max: f64,
    mean: f64,
    samples: usize,
}

/// Nearest-rank quantiles of `samples_ns` (no interpolation).
fn percentiles(mut samples_ns: Vec<f64>) -> Percentiles {
    samples_ns.sort_by(|a, b| a.partial_cmp(b).expect("INVARIANT: no NaN timings"));
    let n = samples_ns.len();
    let at = |q: f64| samples_ns[((n as f64 * q) as usize).min(n - 1)];
    Percentiles {
        p50: at(0.50),
        p99: at(0.99),
        p999: at(0.999),
        max: samples_ns[n - 1],
        mean: samples_ns.iter().sum::<f64>() / n as f64,
        samples: n,
    }
}

fn involuntary_switches() -> i64 {
    // SAFETY: getrusage writes the zeroed rusage it is handed.
    unsafe {
        let mut usage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_THREAD, &mut usage);
        usage.ru_nivcsw
    }
}

#[derive(Default)]
struct Rows {
    gate: Vec<(String, f64)>,
}

impl Rows {
    fn record(&mut self, name: &str, stats: &Percentiles, switches: i64) {
        println!(
            "{name:<24} n={:<7} mean={:>10.0}ns p50={:>10.0}ns p99={:>10.0}ns \
             p99.9={:>10.0}ns max={:>11.0}ns nivcsw={switches}",
            stats.samples, stats.mean, stats.p50, stats.p99, stats.p999, stats.max
        );
        self.gate.push((format!("gw_tail/{name}/p50"), stats.p50));
        self.gate.push((format!("gw_tail/{name}/p99"), stats.p99));
    }

    fn write_gate_file(&self) {
        let dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"))
            .join("tail");
        std::fs::create_dir_all(&dir).expect("create target/tail");
        let mut json = String::from("{\n");
        for (index, (name, ns)) in self.gate.iter().enumerate() {
            let comma = if index + 1 == self.gate.len() {
                ""
            } else {
                ","
            };
            writeln!(json, "  \"{name}\": {ns:.1}{comma}").expect("write to String");
        }
        json.push_str("}\n");
        let path = dir.join("rsx-gateway.json");
        std::fs::write(&path, json).expect("write gate file");
        println!("\ngate entries: {}", path.display());
    }
}

/// Samples per row: `GW_TAIL_SAMPLES`, else `DEFAULT_SAMPLES`. A sample is
/// one round trip, so a loaded box (milliseconds per sample) wants fewer.
fn sample_count() -> usize {
    std::env::var("GW_TAIL_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SAMPLES)
}

/// Reactor core and the recv thread's: the next core when the box has two or
/// more. `GW_TAIL_RECV_CORE` overrides it with a core index, or `none` to
/// leave the recv thread unpinned.
fn cores() -> (usize, Option<usize>) {
    let count = std::thread::available_parallelism().map_or(1, |n| n.get());
    let (reactor, recv) = match count {
        0 | 1 => (0, None),
        2 | 3 => (0, Some(1)),
        _ => (2, Some(3)),
    };
    match std::env::var("GW_TAIL_RECV_CORE") {
        Ok(value) => (reactor, value.parse().ok()),
        Err(_) => (reactor, recv),
    }
}

/// Warm up with `count / 5` calls of `one`, then time up to `count` of them
/// (nanoseconds each), both cut off by their wall-clock budgets. Returns the
/// samples and the involuntary context switches of this thread over them.
async fn sample(count: usize, mut one: impl AsyncFnMut() -> f64) -> (Vec<f64>, i64) {
    let warmup_end = Instant::now() + WARMUP_BUDGET;
    for _ in 0..count / 5 {
        one().await;
        if Instant::now() >= warmup_end {
            break;
        }
    }
    let switches = involuntary_switches();
    let row_end = Instant::now() + ROW_BUDGET;
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        samples.push(one().await);
        if Instant::now() >= row_end {
            break;
        }
    }
    (samples, involuntary_switches() - switches)
}

/// Run `body` to completion on a fresh monoio runtime on this thread.
fn run<F: Future>(body: F) -> F::Output {
    monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .expect("monoio runtime")
        .block_on(body)
}

fn symbol_configs() -> Vec<SymbolConfig> {
    vec![SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }]
}

/// The gateway state with every limiter out of the way: the rows measure the
/// accept path, not a refusal.
fn gateway_state() -> Rc<RefCell<GatewayState>> {
    let mut state = GatewayState::new(10_000, 10, 30_000, build_table(symbol_configs()));
    state.rate_limit_per_user = u32::MAX;
    state.rate_limit_per_ip = u32::MAX;
    state.set_instance_rate_limit(u32::MAX);
    Rc::new(RefCell::new(state))
}

/// A `CastSender` holds a 64 KiB packet buffer inline, so it is built in its
/// own frame and handed on behind a pointer.
fn shared_sender(destination: SocketAddr, wal_dir: &Path) -> Rc<RefCell<CastSender>> {
    Rc::new(RefCell::new(
        CastSender::new(destination, 0, wal_dir).expect("cast sender"),
    ))
}

fn boxed_sender(destination: SocketAddr, wal_dir: &Path) -> Box<CastSender> {
    Box::new(CastSender::new(destination, 0, wal_dir).expect("cast sender"))
}

fn boxed_receiver(sender: SocketAddr) -> Box<CastReceiver> {
    Box::new(CastReceiver::new("127.0.0.1:0".parse().unwrap(), sender).expect("cast receiver"))
}

async fn accept_loop(
    listener: TcpListener,
    state: Rc<RefCell<GatewayState>>,
    sender: Rc<RefCell<CastSender>>,
) {
    loop {
        let (stream, peer) = listener.accept().await.expect("accept");
        stream.set_nodelay(true).expect("nodelay");
        monoio::spawn(handle_connection(
            stream,
            peer,
            state.clone(),
            sender.clone(),
            SECRET,
        ));
    }
}

/// Start the listener on the current runtime; returns its address.
fn serve(state: &Rc<RefCell<GatewayState>>, sender: Rc<RefCell<CastSender>>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind gateway");
    let addr = listener.local_addr().expect("gateway addr");
    monoio::spawn(accept_loop(listener, state.clone(), sender));
    addr
}

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    received: Vec<u8>,
}

impl Client {
    /// Upgrade with a fresh token for `USER_ID` and consume the greeting
    /// heartbeat.
    async fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream.set_nodelay(true).expect("nodelay");
        let request = format!(
            "GET /ws/private HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: {SYNC_KEY}\r\n\
Sec-WebSocket-Version: 13\r\n\
Authorization: Bearer {}\r\n\r\n",
            token(USER_ID)
        );
        let (written, _) = stream.write_all(request.into_bytes()).await;
        written.expect("write upgrade");
        let head = read_head(&mut stream).await;
        assert!(head.starts_with("HTTP/1.1 101"), "upgrade refused: {head}");
        let mut client = Self {
            stream,
            buf: vec![0u8; 4096],
            received: Vec::with_capacity(4096),
        };
        let greeting = client.recv_text().await;
        assert!(greeting.starts_with(r#"{"H":["#), "greeting was {greeting}");
        client
    }

    /// Read one whole server text frame; returns its payload.
    async fn recv_text(&mut self) -> String {
        loop {
            if let Some((header_len, payload_len)) = frame_extent(&self.received) {
                if self.received.len() >= header_len + payload_len {
                    let payload = self.received[header_len..header_len + payload_len].to_vec();
                    self.received.drain(..header_len + payload_len);
                    return String::from_utf8(payload).expect("utf8 frame");
                }
            }
            let buf = std::mem::take(&mut self.buf);
            let (result, buf) = self.stream.read(buf).await;
            let len = result.expect("client read");
            assert!(len > 0, "gateway closed the connection");
            self.received.extend_from_slice(&buf[..len]);
            self.buf = buf;
        }
    }
}

/// Header and payload length of the unmasked server frame at the start of
/// `bytes`, once enough of the header has arrived.
fn frame_extent(bytes: &[u8]) -> Option<(usize, usize)> {
    let short = *bytes.get(1)? & 0x7F;
    match short {
        126 => Some((
            4,
            u16::from_be_bytes([*bytes.get(2)?, *bytes.get(3)?]) as usize,
        )),
        127 => None,
        short => Some((2, short as usize)),
    }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut buf = vec![0u8; 1024];
    while !head.ends_with(b"\r\n\r\n") {
        let (result, returned) = stream.read(buf).await;
        let len = result.expect("read upgrade response");
        assert!(len > 0, "gateway closed during the upgrade");
        head.extend_from_slice(&returned[..len]);
        buf = returned;
    }
    String::from_utf8(head).expect("utf8 head")
}

/// A masked text frame, as a browser sends.
fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x81];
    if payload.len() <= 125 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&MASK_KEY);
    for (index, byte) in payload.iter().enumerate() {
        frame.push(byte ^ MASK_KEY[index % 4]);
    }
    frame
}

fn token(user_id: u32) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    mint_jwt(&Claims::for_user(user_id, now + 3600), SECRET).expect("jwt encode")
}

/// A GTC limit buy on symbol 0 with a unique 20-byte client order id.
fn order_frame(n: u64) -> Vec<u8> {
    encode_frame(format!(r#"{{"N":[0,0,1000,10,"{n:020}",0]}}"#).as_bytes())
}

struct Order {
    risk: monoio::net::udp::UdpSocket,
    state: Rc<RefCell<GatewayState>>,
    client: Client,
    datagram: Vec<u8>,
    next: u64,
}

impl Order {
    /// Send one order frame and return the nanoseconds until its
    /// `OrderRequest` is at the risk socket.
    async fn timed(&mut self) -> f64 {
        self.next += 1;
        let frame = order_frame(self.next);
        let start = Instant::now();
        let (written, _) = self.client.stream.write_all(frame).await;
        let (received, returned) = self
            .risk
            .recv_from(std::mem::take(&mut self.datagram))
            .await;
        let ns = start.elapsed().as_nanos() as f64;
        self.datagram = returned;
        written.expect("write order");
        let (len, _) = received.expect("risk recv");
        let header = WalHeader::from_bytes(&self.datagram[..len]).expect("wal header");
        assert_eq!(
            header.record_type, RECORD_ORDER_REQUEST,
            "expected an order"
        );
        let order: OrderRequest =
            decode_payload(&self.datagram[WalHeader::SIZE..len]).expect("order payload");
        let order_id = join_order_id(order.order_id_hi, order.order_id_lo);
        let mut state = self.state.borrow_mut();
        state
            .pending
            .remove(order.user_id, &order_id)
            .expect("pending entry for the forwarded order");
        if self.next.is_multiple_of(CLEAN_EVERY) {
            state.client_ids.expire(u64::MAX);
        }
        ns
    }
}

async fn bench_order_path(rows: &mut Rows, count: usize) {
    let wal_dir = tempfile::tempdir().expect("tempdir");
    let risk_std = UdpSocket::bind("127.0.0.1:0").expect("bind risk");
    risk_std.set_nonblocking(true).expect("nonblocking");
    let risk_addr = risk_std.local_addr().expect("risk addr");

    let state = gateway_state();
    let addr = serve(&state, shared_sender(risk_addr, wal_dir.path()));
    let mut order = Order {
        risk: monoio::net::udp::UdpSocket::from_std(risk_std).expect("monoio risk socket"),
        state,
        client: Client::connect(addr).await,
        datagram: vec![0u8; 2048],
        next: 0,
    };

    let (samples, switches) = sample(count, async || order.timed().await).await;
    rows.record("order_accept", &percentiles(samples), switches);
}

/// The reactor's drain-and-route step from `src/main.rs`, woken by the recv
/// thread's doorbell.
async fn route_events(
    mut events: rtrb::Consumer<CastEvent>,
    doorbell: UdpSocket,
    state: Rc<RefCell<GatewayState>>,
) {
    let readiness = monoio::net::udp::UdpSocket::from_std(doorbell.try_clone().expect("dup"))
        .expect("monoio doorbell");
    loop {
        let mut ping = [0u8; 64];
        while doorbell.recv(&mut ping).is_ok() {}
        while let Ok(event) = events.pop() {
            match event {
                CastEvent::Fill(record) => route_fill(&state, &record),
                CastEvent::OrderDone(record) => route_order_done(&state, &record),
                CastEvent::OrderInserted(record) => route_order_inserted(&state, &record),
                _ => unreachable!("the route rows send fills, inserts and dones"),
            }
        }
        monoio::select! {
            ready = readiness.readable(false) => ready.expect("doorbell readable"),
            _ = monoio::time::sleep(HOUSEKEEPING_INTERVAL) => {}
        }
    }
}

struct Route {
    risk: Box<CastSender>,
    state: Rc<RefCell<GatewayState>>,
    client: Client,
    seq: u64,
    next_id: u64,
    count: usize,
}

impl Route {
    /// Send the record `make` builds for a fresh pending order and return the
    /// nanoseconds until its frame, starting `tag`, has been read.
    async fn timed<T: CastRecord>(&mut self, tag: &str, make: impl Fn(u64) -> T) -> f64 {
        self.next_id += 1;
        self.seq += 1;
        let order_id = join_order_id(0, self.next_id);
        self.state.borrow_mut().pending.push(PendingOrder {
            order_id,
            user_id: USER_ID,
            symbol_id: 0,
            client_order_id: [0; 20],
            timestamp_ns: 0,
            stage: PendingStage::Submitted,
        });
        let mut record = make(self.next_id);
        record.set_seq(self.seq);
        let start = Instant::now();
        self.risk
            .send_raw(T::record_type(), as_bytes(&record))
            .expect("send record");
        let frame = self.client.recv_text().await;
        let ns = start.elapsed().as_nanos() as f64;
        assert!(frame.starts_with(tag), "expected {tag}, got {frame}");
        self.state.borrow_mut().pending.remove(USER_ID, &order_id);
        black_box(frame);
        ns
    }

    async fn row<T: CastRecord>(
        &mut self,
        rows: &mut Rows,
        name: &str,
        tag: &str,
        make: impl Fn(u64) -> T,
    ) {
        let count = self.count;
        let (samples, switches) = sample(count, async || self.timed(tag, &make).await).await;
        rows.record(name, &percentiles(samples), switches);
    }
}

fn inserted(id: u64) -> OrderInsertedRecord {
    OrderInsertedRecord {
        seq: 0,
        ts_ns: time_ns(),
        symbol_id: 0,
        user_id: USER_ID,
        order_id_hi: 0,
        order_id_lo: id,
        price: Price(1000),
        qty: Qty(10),
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    }
}

fn done(id: u64) -> OrderDoneRecord {
    let now = time_ns();
    OrderDoneRecord {
        seq: 0,
        ts_ns: now,
        symbol_id: 0,
        user_id: USER_ID,
        order_id_hi: 0,
        order_id_lo: id,
        filled_qty: Qty(10),
        remaining_qty: Qty(0),
        final_status: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: now,
        risk_in_ns: now,
        me_in_ns: now,
        match_done_ns: now,
        gw_out_ns: 0,
        _pad2: [0; 24],
    }
}

fn fill(id: u64) -> FillRecord {
    let now = time_ns();
    FillRecord {
        seq: 0,
        ts_ns: now,
        symbol_id: 0,
        taker_user_id: USER_ID,
        maker_user_id: MAKER_USER_ID,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: id,
        maker_order_id_hi: 0,
        maker_order_id_lo: id + (1 << 40),
        price: Price(1000),
        qty: Qty(10),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: now,
        risk_in_ns: now,
        me_in_ns: now,
        match_done_ns: now,
        gw_out_ns: 0,
    }
}

async fn bench_route_path(rows: &mut Rows, recv_core: Option<usize>, count: usize) {
    let wal_dir = tempfile::tempdir().expect("tempdir");
    // Nothing reads this socket: it takes the gateway sender's datagrams and
    // the receiver's control traffic.
    let orders = UdpSocket::bind("127.0.0.1:0").expect("bind orders");
    let orders_addr = orders.local_addr().expect("orders addr");
    let receiver = boxed_receiver(orders_addr);
    let risk = boxed_sender(
        receiver.local_addr().expect("receiver addr"),
        wal_dir.path(),
    );

    let state = gateway_state();
    let addr = serve(&state, shared_sender(orders_addr, wal_dir.path()));
    let wal_path = wal_dir.path().to_string_lossy().into_owned();
    let handle = cast_recv::spawn(*receiver, wal_path, &SHUTDOWN, RING_CAPACITY, recv_core)
        .expect("spawn cast-recv");
    monoio::spawn(route_events(
        handle.consumer,
        handle.doorbell,
        state.clone(),
    ));

    let mut route = Route {
        risk,
        state,
        client: Client::connect(addr).await,
        seq: 0,
        next_id: 0,
        count,
    };
    route
        .row(rows, "route_inserted", r#"{"U":["#, inserted)
        .await;
    route.row(rows, "route_done", r#"{"U":["#, done).await;
    route.row(rows, "route_fill", r#"{"F":["#, fill).await;

    SHUTDOWN.store(true, Ordering::SeqCst);
    handle.join.join().expect("cast-recv thread");
    drop(orders);
}

fn main() {
    let (reactor_core, recv_core) = cores();
    let count = sample_count();
    assert!(
        rsx_types::cpu::pin_current(reactor_core),
        "pin the reactor thread to core {reactor_core}"
    );
    let mut rows = Rows::default();

    let floor = percentiles(
        (0..FLOOR_SAMPLES)
            .map(|_| {
                let start = Instant::now();
                black_box(Instant::now()).duration_since(start).as_nanos() as f64
            })
            .collect(),
    );
    println!("timer floor: back-to-back Instant::now(), n={FLOOR_SAMPLES}");
    println!(
        "timer_floor p50={:.0}ns p99={:.0}ns\n",
        floor.p50, floor.p99
    );
    println!("reactor core {reactor_core}, cast-recv core {recv_core:?}\n");

    run(bench_order_path(&mut rows, count));
    run(bench_route_path(&mut rows, recv_core, count));

    rows.write_gate_file();
}
