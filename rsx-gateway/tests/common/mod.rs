//! Connection-handler harness: the real `handle_connection` on a monoio
//! loopback listener, a WebSocket client that sends masked RFC 6455 frames,
//! and risk faked only at the casting/UDP boundary. A UDP socket records what
//! the gateway sends; a `CastSender` injects risk responses into the
//! gateway's real `CastReceiver`, decoded with `decode_cast_event` and routed
//! with the `route_*` functions the way the event match in `src/main.rs` does.
//! The binary's housekeeping loop is not driven: tests call the functions it
//! calls (`expire_stale_orders`, `broadcast_heartbeat`,
//! `reap_idle_connections`) themselves.
#![allow(dead_code)] // each test binary compiles this module and uses a subset

pub mod replay;

use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use monoio::io::AsyncReadRent;
use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use rsx_cast::as_bytes;
use rsx_cast::decode_payload;
use rsx_cast::CastReceiver;
use rsx_cast::CastRecord;
use rsx_cast::CastRecvWith;
use rsx_cast::CastSender;
use rsx_cast::WalHeader;
use rsx_gateway::cast_recv::decode_cast_event;
use rsx_gateway::cast_recv::CastEvent;
use rsx_gateway::handler::handle_connection;
use rsx_gateway::jwt::Claims;
use rsx_gateway::order_id::order_id_to_hex;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_liquidation;
use rsx_gateway::route::route_order_cancelled;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_failed;
use rsx_gateway::route::route_order_inserted;
use rsx_gateway::state::GatewayState;
use rsx_gateway::state::DEFAULT_MAX_OUTBOUND;
use rsx_gateway::state::DEFAULT_RATE_LIMIT_PER_INSTANCE;
use rsx_messages::CancelRequest;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_risk::types::OrderRequest;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use std::cell::Cell;
use std::cell::RefCell;
use std::future::Future;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

pub const SECRET: &str = "handler-test-secret-padded-to-32-bytes!";
pub const AUDIENCE: &str = "rsx-gateway";
pub const ISSUER: &str = "rsx-auth";
pub const HEARTBEAT_ECHO: &str = r#"{"H":[0]}"#;

const WAIT: Duration = Duration::from_secs(3);
const MASK_KEY: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
const SYNC_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

pub struct Options {
    pub max_pending: usize,
    pub max_outbound: usize,
    pub circuit_threshold: u32,
    pub circuit_cooldown_ms: u64,
    pub rate_limit_per_user: u32,
    pub rate_limit_per_ip: u32,
    pub rate_limit_per_instance: u32,
    pub risk_reachable: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_pending: 100,
            max_outbound: DEFAULT_MAX_OUTBOUND,
            circuit_threshold: 10,
            circuit_cooldown_ms: 30_000,
            rate_limit_per_user: 1000,
            rate_limit_per_ip: 1000,
            rate_limit_per_instance: DEFAULT_RATE_LIMIT_PER_INSTANCE,
            risk_reachable: true,
        }
    }
}

/// Symbol 0 has tick 10 and lot 5; symbol 1 has tick 1 and lot 1.
fn symbol_configs() -> Vec<SymbolConfig> {
    vec![
        SymbolConfig {
            symbol_id: 0,
            price_decimals: 0,
            qty_decimals: 0,
            tick_size: 10,
            lot_size: 5,
        },
        SymbolConfig {
            symbol_id: 1,
            price_decimals: 0,
            qty_decimals: 0,
            tick_size: 1,
            lot_size: 1,
        },
    ]
}

/// Run `body` against a fresh gateway on a monoio runtime on the test thread,
/// so a failed assertion fails only its own test.
pub fn run<F, Fut>(options: Options, body: F)
where
    F: FnOnce(Gateway) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();
    runtime.block_on(async move { body(Gateway::start(options)).await });
}

/// A `CastSender` and a `CastReceiver` each hold a 64 KiB packet buffer
/// inline, so they are built in their own frames and handed on behind a
/// pointer; a test thread's stack does not take several of them by value.
fn shared_sender(destination: SocketAddr, wal_dir: &Path) -> Rc<RefCell<CastSender>> {
    Rc::new(RefCell::new(
        CastSender::new(destination, 0, wal_dir).unwrap(),
    ))
}

fn boxed_sender(destination: SocketAddr, wal_dir: &Path) -> Box<CastSender> {
    Box::new(CastSender::new(destination, 0, wal_dir).unwrap())
}

fn boxed_receiver(sender: SocketAddr) -> Box<CastReceiver> {
    Box::new(CastReceiver::new("127.0.0.1:0".parse().unwrap(), sender).unwrap())
}

pub struct Gateway {
    pub addr: SocketAddr,
    pub state: Rc<RefCell<GatewayState>>,
    pub risk: Risk,
    _wal_dir: TempDir,
}

impl Gateway {
    fn start(options: Options) -> Self {
        let wal_dir = tempfile::tempdir().unwrap();
        let orders = UdpSocket::bind("127.0.0.1:0").unwrap();
        orders.set_nonblocking(true).unwrap();
        let orders_addr = orders.local_addr().unwrap();
        let destination = if options.risk_reachable {
            orders_addr
        } else {
            "127.0.0.1:0".parse().unwrap()
        };
        let sender = shared_sender(destination, wal_dir.path());
        let receiver = boxed_receiver(orders_addr);
        let responses = boxed_sender(receiver.local_addr().unwrap(), wal_dir.path());

        let mut gateway_state = GatewayState::new(
            options.max_pending,
            options.circuit_threshold,
            options.circuit_cooldown_ms,
            symbol_configs(),
        );
        gateway_state.rate_limit_per_user = options.rate_limit_per_user;
        gateway_state.rate_limit_per_ip = options.rate_limit_per_ip;
        gateway_state.set_instance_rate_limit(options.rate_limit_per_instance);
        gateway_state.max_outbound = options.max_outbound;
        let state = Rc::new(RefCell::new(gateway_state));

        let delivered = Rc::new(Cell::new(0));
        monoio::spawn(deliver_responses(
            receiver,
            state.clone(),
            delivered.clone(),
        ));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        monoio::spawn(accept_loop(listener, state.clone(), sender));

        Self {
            addr,
            state,
            risk: Risk {
                orders,
                responses: RefCell::new(responses),
                next_seq: Cell::new(1),
                delivered,
            },
            _wal_dir: wal_dir,
        }
    }

    /// Send `frame` as `client` once it is fully handled and return the one
    /// order record risk received, with no reply to the client.
    pub async fn place(&self, client: &mut Client, frame: &str) -> OrderRequest {
        client.send_text(frame).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        self.risk.take_order()
    }

    pub fn connection_count(&self) -> usize {
        self.state.borrow().connections.len()
    }

    /// Open a connection with a fresh valid token for `user_id` and consume
    /// the greeting heartbeat.
    pub async fn connect(&self, user_id: u32) -> Client {
        let (head, mut client) = self.upgrade(Some(&token(user_id))).await;
        assert!(head.starts_with("HTTP/1.1 101"), "upgrade refused: {head}");
        let greeting = client.recv().await;
        assert!(greeting.starts_with(r#"{"H":["#), "greeting was {greeting}");
        client
    }

    /// Send a WebSocket upgrade request and return the response head with the
    /// socket.
    pub async fn upgrade(&self, token: Option<&str>) -> (String, Client) {
        match token {
            Some(token) => {
                let header = format!("Authorization: Bearer {token}");
                self.upgrade_with_header(&header).await
            }
            None => self.upgrade_with_header("").await,
        }
    }

    /// Like `upgrade` with a caller-chosen extra header line (may be empty).
    pub async fn upgrade_with_header(&self, header: &str) -> (String, Client) {
        self.send_upgrade(&[&upgrade_request(header)]).await
    }

    /// Upgrade with a valid token whose Authorization header arrives in a
    /// second TCP write, 50 ms after the rest of the request.
    pub async fn upgrade_in_two_writes(&self, token: &str) -> (String, Client) {
        let request = upgrade_request(&format!("Authorization: Bearer {token}"));
        let split = request.find("Authorization").unwrap();
        self.send_upgrade(&[&request[..split], &request[split..]])
            .await
    }

    async fn send_upgrade(&self, chunks: &[&str]) -> (String, Client) {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                monoio::time::sleep(Duration::from_millis(50)).await;
            }
            let (written, _) = stream.write_all(chunk.as_bytes().to_vec()).await;
            written.unwrap();
        }
        let head = read_head(&mut stream).await;
        (head, Client { stream })
    }

    /// Wait until `condition` holds, failing the test after a deadline.
    pub async fn wait_until(&self, what: &str, condition: impl Fn(&Gateway) -> bool) {
        let deadline = std::time::Instant::now() + WAIT;
        while !condition(self) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            monoio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

fn upgrade_request(header: &str) -> String {
    let header = if header.is_empty() {
        String::new()
    } else {
        format!("{header}\r\n")
    };
    format!(
        "GET /ws/private HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: {SYNC_KEY}\r\n\
Sec-WebSocket-Version: 13\r\n\
{header}\r\n"
    )
}

async fn accept_loop(
    listener: TcpListener,
    state: Rc<RefCell<GatewayState>>,
    sender: Rc<RefCell<CastSender>>,
) {
    loop {
        let (stream, peer) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        monoio::spawn(handle_connection(
            stream,
            peer,
            state.clone(),
            sender.clone(),
            SECRET,
        ));
    }
}

async fn deliver_responses(
    mut receiver: Box<CastReceiver>,
    state: Rc<RefCell<GatewayState>>,
    delivered: Rc<Cell<usize>>,
) {
    loop {
        loop {
            let outcome = receiver.try_recv_with(|header, payload| {
                if let Some(event) = decode_cast_event(header.record_type, payload) {
                    route_event(&state, event);
                }
                delivered.set(delivered.get() + 1);
            });
            match outcome {
                CastRecvWith::Data => {}
                CastRecvWith::Empty => break,
                other => panic!("risk response stream broke: {other:?}"),
            }
        }
        monoio::time::sleep(Duration::from_millis(1)).await;
    }
}

pub fn route_event(state: &Rc<RefCell<GatewayState>>, event: CastEvent) {
    match event {
        CastEvent::Fill(record) => route_fill(state, &record),
        CastEvent::OrderDone(record) => route_order_done(state, &record),
        CastEvent::OrderCancelled(record) => route_order_cancelled(state, &record),
        CastEvent::OrderInserted(record) => route_order_inserted(state, &record),
        CastEvent::OrderFailed(record) => route_order_failed(state, &record),
        CastEvent::Liquidation(record) => route_liquidation(state, &record),
        CastEvent::ConfigApplied {
            symbol_id,
            config_version,
        } => {
            state
                .borrow_mut()
                .apply_config_applied(symbol_id, config_version);
        }
    }
}

/// Risk as the gateway sees it over casting/UDP.
pub struct Risk {
    orders: UdpSocket,
    responses: RefCell<Box<CastSender>>,
    next_seq: Cell<u64>,
    delivered: Rc<Cell<usize>>,
}

impl Risk {
    /// Every datagram the gateway sent since the previous call, as record
    /// type and payload. Loopback UDP is delivered before the gateway's
    /// `send_to` returns, so a frame the gateway has answered is already here.
    pub fn take(&self) -> Vec<(u16, Vec<u8>)> {
        let mut datagrams = Vec::new();
        let mut buf = [0u8; 2048];
        while let Ok(len) = self.orders.recv(&mut buf) {
            let header = WalHeader::from_bytes(&buf[..len]).unwrap();
            let end = WalHeader::SIZE + header.len as usize;
            datagrams.push((header.record_type, buf[WalHeader::SIZE..end].to_vec()));
        }
        datagrams
    }

    pub fn take_orders(&self) -> Vec<OrderRequest> {
        self.take()
            .into_iter()
            .map(|(record_type, payload)| {
                assert_eq!(record_type, RECORD_ORDER_REQUEST, "expected an order");
                decode_payload::<OrderRequest>(&payload).unwrap()
            })
            .collect()
    }

    pub fn take_order(&self) -> OrderRequest {
        let mut orders = self.take_orders();
        assert_eq!(orders.len(), 1, "expected exactly one order");
        orders.remove(0)
    }

    pub fn take_cancels(&self) -> Vec<CancelRequest> {
        self.take()
            .into_iter()
            .map(|(record_type, payload)| {
                assert_eq!(record_type, RECORD_CANCEL_REQUEST, "expected a cancel");
                decode_payload::<CancelRequest>(&payload).unwrap()
            })
            .collect()
    }

    pub fn assert_silent(&self) {
        let datagrams = self.take();
        assert!(
            datagrams.is_empty(),
            "gateway sent {} unexpected datagrams to risk",
            datagrams.len()
        );
    }

    /// Send a response record to the gateway and wait until it has been
    /// routed to the connections' outboxes.
    pub async fn inject<T: CastRecord>(&self, mut record: T) {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        record.set_seq(seq);
        let before = self.delivered.get();
        self.responses
            .borrow_mut()
            .send_raw(T::record_type(), as_bytes(&record))
            .unwrap();
        let deadline = std::time::Instant::now() + WAIT;
        while self.delivered.get() == before {
            assert!(
                std::time::Instant::now() < deadline,
                "gateway did not receive the injected record"
            );
            monoio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

pub struct Client {
    stream: TcpStream,
}

impl Client {
    pub async fn send_text(&mut self, text: &str) {
        self.send_frame(true, 0x1, true, text.as_bytes()).await;
    }

    pub async fn send_frame(&mut self, fin: bool, opcode: u8, masked: bool, payload: &[u8]) {
        let bytes = encode_frame(fin, opcode, masked, payload);
        let (written, _) = self.stream.write_all(bytes).await;
        written.unwrap();
    }

    /// Send the first bytes of a masked text frame, leaving it incomplete.
    pub async fn send_partial_frame(&mut self, text: &str) {
        let bytes = encode_frame(true, 0x1, true, text.as_bytes());
        let (written, _) = self.stream.write_all(bytes[..3].to_vec()).await;
        written.unwrap();
    }

    /// Fail unless the server stays silent for a short while.
    pub async fn assert_quiet(&mut self) {
        let quiet = Duration::from_millis(150);
        if let Ok((opcode, payload)) =
            monoio::time::timeout(quiet, read_frame(&mut self.stream)).await
        {
            panic!(
                "unexpected frame: opcode {opcode} {:?}",
                String::from_utf8_lossy(&payload)
            );
        }
    }

    /// Read one server frame as opcode and payload.
    pub async fn recv_frame(&mut self) -> (u8, Vec<u8>) {
        match monoio::time::timeout(WAIT, read_frame(&mut self.stream)).await {
            Ok(frame) => frame,
            Err(_) => panic!("timed out waiting for a server frame"),
        }
    }

    /// Read one server text frame.
    pub async fn recv(&mut self) -> String {
        let (opcode, payload) = self.recv_frame().await;
        assert_eq!(opcode, 0x1, "expected a text frame");
        String::from_utf8(payload).unwrap()
    }

    /// Round-trip two heartbeats so that everything the gateway queued for
    /// this connection before the call has been written, and return those
    /// frames. The handler serves one connection sequentially, so the second
    /// echo follows every earlier write.
    pub async fn settle(&mut self) -> Vec<String> {
        let mut frames = Vec::new();
        for _ in 0..2 {
            self.send_text(HEARTBEAT_ECHO).await;
            loop {
                let frame = self.recv().await;
                if frame.starts_with(r#"{"H":["#) {
                    break;
                }
                frames.push(frame);
            }
        }
        frames
    }

    /// Send `frame` and expect exactly the error `E[code, message]`, with
    /// the connection still open and nothing else queued.
    pub async fn reject(&mut self, frame: &str, code: u32, message: &str) {
        self.send_text(frame).await;
        assert_eq!(self.recv().await, error_frame(code, message));
        assert_eq!(self.settle().await, Vec::<String>::new());
    }

    /// Read until the server closes the socket and return the bytes that
    /// arrived first. A reset counts as closed.
    pub async fn read_to_close(&mut self) -> Vec<u8> {
        match monoio::time::timeout(WAIT, read_until_closed(&mut self.stream)).await {
            Ok(bytes) => bytes,
            Err(_) => panic!("socket still open after {WAIT:?}"),
        }
    }
}

async fn read_until_closed(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    loop {
        let (result, buf) = stream.read(vec![0u8; 4096]).await;
        match result {
            Ok(0) | Err(_) => return received,
            Ok(len) => received.extend_from_slice(&buf[..len]),
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let (result, byte) = stream.read_exact(vec![0u8; 1]).await;
        if result.is_err() {
            break;
        }
        head.extend_from_slice(&byte);
    }
    String::from_utf8(head).unwrap()
}

async fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let (result, preamble) = stream.read_exact(vec![0u8; 2]).await;
    result.unwrap();
    let opcode = preamble[0] & 0x0F;
    let length = match preamble[1] & 0x7F {
        126 => {
            let (result, ext) = stream.read_exact(vec![0u8; 2]).await;
            result.unwrap();
            u16::from_be_bytes([ext[0], ext[1]]) as usize
        }
        127 => {
            let (result, ext) = stream.read_exact(vec![0u8; 8]).await;
            result.unwrap();
            u64::from_be_bytes(ext.try_into().unwrap()) as usize
        }
        short => short as usize,
    };
    let (result, payload) = stream.read_exact(vec![0u8; length]).await;
    result.unwrap();
    (opcode, payload)
}

pub fn encode_frame(fin: bool, opcode: u8, masked: bool, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![(if fin { 0x80 } else { 0 }) | opcode];
    let mask_bit = if masked { 0x80 } else { 0 };
    if payload.len() <= 125 {
        frame.push(mask_bit | payload.len() as u8);
    } else if payload.len() <= 65535 {
        frame.push(mask_bit | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    if masked {
        frame.extend_from_slice(&MASK_KEY);
        for (index, byte) in payload.iter().enumerate() {
            frame.push(byte ^ MASK_KEY[index % 4]);
        }
    } else {
        frame.extend_from_slice(payload);
    }
    frame
}

pub fn error_frame(code: u32, message: &str) -> String {
    format!(r#"{{"E":[{code},"{message}"]}}"#)
}

/// The array under the frame's single key `tag`, e.g. `U` or `F`.
pub fn fields(frame: &str, tag: &str) -> Vec<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(frame).unwrap();
    value[tag]
        .as_array()
        .unwrap_or_else(|| panic!("not a {tag} frame: {frame}"))
        .clone()
}

/// The code of an `E` frame, whatever its message.
pub fn error_code(frame: &str) -> u64 {
    fields(frame, "E")[0].as_u64().unwrap()
}

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A valid gateway token for `user_id` with a jti no other token has used.
pub fn token(user_id: u32) -> String {
    mint_token(user_id, AUDIENCE, ISSUER, SECRET)
}

pub fn mint_token(user_id: u32, audience: &str, issuer: &str, secret: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = Claims {
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp: now + 3600,
        aud: Some(audience.to_string()),
        iss: Some(issuer.to_string()),
        nbf: None,
        jti: Some(format!(
            "handler-test-{}-{}",
            std::process::id(),
            TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
        )),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

/// A 20-byte client order id.
pub fn cid(n: u32) -> String {
    format!("{n:020}")
}

/// A valid limit order on symbol 0 (tick 10, lot 5): buy 10 at 1000, GTC.
pub fn order(cid: &str) -> String {
    format!(r#"{{"N":[0,0,1000,10,"{cid}",0]}}"#)
}

pub fn cancel(key: &str) -> String {
    format!(r#"{{"C":["{key}"]}}"#)
}

/// The order's id as the 32-hex string the client sees in `U` frames.
pub fn order_hex(order: &OrderRequest) -> String {
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&order.order_id_hi.to_be_bytes());
    id[8..].copy_from_slice(&order.order_id_lo.to_be_bytes());
    order_id_to_hex(&id)
}

/// Owner and id of an order, as risk records carry them.
#[derive(Clone, Copy)]
pub struct Ids {
    pub user_id: u32,
    pub hi: u64,
    pub lo: u64,
}

pub fn ids(order: &OrderRequest) -> Ids {
    Ids {
        user_id: order.user_id,
        hi: order.order_id_hi,
        lo: order.order_id_lo,
    }
}

pub fn inserted(ids: Ids, qty: i64) -> OrderInsertedRecord {
    OrderInsertedRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 0,
        user_id: ids.user_id,
        order_id_hi: ids.hi,
        order_id_lo: ids.lo,
        price: Price(1000),
        qty: Qty(qty),
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    }
}

pub fn done(ids: Ids, filled: i64, remaining: i64, final_status: u8) -> OrderDoneRecord {
    OrderDoneRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 0,
        user_id: ids.user_id,
        order_id_hi: ids.hi,
        order_id_lo: ids.lo,
        filled_qty: Qty(filled),
        remaining_qty: Qty(remaining),
        final_status,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

pub fn cancelled(ids: Ids, remaining: i64, reason: u8) -> OrderCancelledRecord {
    OrderCancelledRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 0,
        user_id: ids.user_id,
        order_id_hi: ids.hi,
        order_id_lo: ids.lo,
        remaining_qty: Qty(remaining),
        reason,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    }
}

pub fn failed(ids: Ids, reason: u8) -> OrderFailedRecord {
    OrderFailedRecord {
        seq: 0,
        ts_ns: 1,
        user_id: ids.user_id,
        _pad0: 0,
        order_id_hi: ids.hi,
        order_id_lo: ids.lo,
        reason,
        _pad: [0; 23],
    }
}

pub fn fill(taker: Ids, maker: Ids, price: i64, qty: i64) -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 7,
        symbol_id: 0,
        taker_user_id: taker.user_id,
        maker_user_id: maker.user_id,
        _pad0: 0,
        taker_order_id_hi: taker.hi,
        taker_order_id_lo: taker.lo,
        maker_order_id_hi: maker.hi,
        maker_order_id_lo: maker.lo,
        price: Price(price),
        qty: Qty(qty),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

pub fn liquidation(user_id: u32) -> LiquidationRecord {
    LiquidationRecord {
        seq: 0,
        ts_ns: 1,
        user_id,
        symbol_id: 0,
        status: 1,
        side: 1,
        _pad0: [0; 2],
        round: 2,
        qty: 30,
        price: 900,
        slip_bps: 15,
    }
}
