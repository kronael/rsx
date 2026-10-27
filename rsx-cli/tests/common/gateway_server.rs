use futures_util::SinkExt;
use futures_util::StreamExt;
use std::net::TcpListener as StdListener;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::runtime::Builder;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::Callback;
use tokio_tungstenite::tungstenite::handshake::server::ErrorResponse;
use tokio_tungstenite::tungstenite::handshake::server::Request;
use tokio_tungstenite::tungstenite::handshake::server::Response;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

const HEARTBEAT_GREETING: &str = r#"{"H":[1]}"#;
const SYMBOLS_REQUEST: &[u8] = b"GET /v1/symbols";

/// What one fake gateway connection does: the greeting heartbeat, the frames
/// sent back after the n-th order or cancel frame it receives (heartbeats
/// from the client do not count), or a refused handshake.
#[derive(Clone)]
pub struct Script {
    pub greeting: bool,
    pub replies: Vec<Vec<String>>,
    pub refuse: bool,
    pub responder: Option<Responder>,
}

/// How a connection answers every new order without a script: after `delay`,
/// the order fills (one `F` and one `U` with server timestamps), or is
/// rejected with `reject_reason` every `reject_every`th order (0 never), or
/// gets the gateway's unknown-symbol `E`. A connection stops answering after
/// `answer_limit` orders.
#[derive(Clone)]
pub struct Responder {
    pub delay: Duration,
    pub reject_every: usize,
    pub reject_reason: u8,
    pub is_unknown_symbol: bool,
    pub answer_limit: Option<usize>,
}

impl Responder {
    pub fn filling() -> Self {
        Self {
            delay: Duration::ZERO,
            reject_every: 0,
            reject_reason: 0,
            is_unknown_symbol: false,
            answer_limit: None,
        }
    }

    pub fn rejecting_every(count: usize, reason: u8) -> Self {
        Self {
            reject_every: count,
            reject_reason: reason,
            ..Self::filling()
        }
    }

    pub fn delayed(delay: Duration) -> Self {
        Self {
            delay,
            ..Self::filling()
        }
    }

    pub fn answering_only(limit: usize) -> Self {
        Self {
            answer_limit: Some(limit),
            ..Self::filling()
        }
    }

    pub fn refusing_unknown_symbol() -> Self {
        Self {
            is_unknown_symbol: true,
            ..Self::filling()
        }
    }

    fn frames(&self, connection: usize, order: usize) -> Vec<String> {
        if self.is_unknown_symbol {
            return vec![r#"{"E":[1007,"unknown symbol"]}"#.to_string()];
        }
        let oid = format!("{connection:016x}{:016x}", order + 1);
        if self.reject_every > 0 && (order + 1).is_multiple_of(self.reject_every) {
            return vec![format!(
                "{{\"U\":[\"{oid}\",3,0,0,{},0,0,0,0,0]}}",
                self.reject_reason
            )];
        }
        let base = 1_700_000_000_000_000_000u64 + order as u64 * 1_000_000;
        let stamps = format!(
            "{},{},{},{},{}",
            base,
            base + 1000,
            base + 3000,
            base + 6000,
            base + 10_000
        );
        vec![
            format!("{{\"F\":[\"{oid}\",\"{oid}\",100,1,5000,0,{stamps}]}}"),
            format!("{{\"U\":[\"{oid}\",0,1,0,0,{stamps}]}}"),
        ]
    }
}

impl Script {
    pub fn answering(replies: &[&[&str]]) -> Self {
        Self {
            greeting: true,
            replies: replies
                .iter()
                .map(|frames| frames.iter().map(|frame| frame.to_string()).collect())
                .collect(),
            refuse: false,
            responder: None,
        }
    }

    pub fn responding(responder: Responder) -> Self {
        Self {
            greeting: true,
            replies: Vec::new(),
            refuse: false,
            responder: Some(responder),
        }
    }

    pub fn refusing() -> Self {
        Self {
            greeting: false,
            replies: Vec::new(),
            refuse: true,
            responder: None,
        }
    }
}

#[derive(Default)]
struct Log {
    texts: Mutex<Vec<(usize, String)>>,
    auth: Mutex<Vec<String>>,
    closed: Mutex<usize>,
    changed: Condvar,
}

pub struct GatewayServer {
    pub url: String,
    log: Arc<Log>,
}

impl GatewayServer {
    /// Every text frame clients sent, as (connection index, text) in arrival
    /// order, heartbeats included.
    pub fn texts(&self) -> Vec<(usize, String)> {
        self.log.texts.lock().unwrap().clone()
    }

    /// The Authorization header of each WebSocket connection, in order.
    pub fn auth(&self) -> Vec<String> {
        self.log.auth.lock().unwrap().clone()
    }

    /// Blocks until `count` WebSocket connections have closed.
    pub fn wait_closed(&self, count: usize) {
        let mut closed = self.log.closed.lock().unwrap();
        while *closed < count {
            let (guard, timeout) = self
                .log
                .changed
                .wait_timeout(closed, Duration::from_secs(5))
                .unwrap();
            closed = guard;
            assert!(!timeout.timed_out(), "connections still open: {}", *closed);
        }
    }
}

/// A url nothing listens on.
pub fn closed_url() -> String {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    format!("ws://{}", listener.local_addr().unwrap())
}

/// Starts a fake gateway. WebSocket connection i follows `scripts[i]` (the
/// last script repeats past the end); `GET /v1/symbols` answers `symbols`
/// as the JSON body, 404 when `None`.
pub fn serve(scripts: Vec<Script>, symbols: Option<&str>) -> GatewayServer {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let log = Arc::new(Log::default());
    let shared = log.clone();
    let symbols = symbols.map(str::to_string);
    thread::spawn(move || {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(accept_loop(listener, scripts, symbols, shared));
    });
    GatewayServer { url, log }
}

async fn accept_loop(
    listener: StdListener,
    scripts: Vec<Script>,
    symbols: Option<String>,
    log: Arc<Log>,
) {
    let listener = TcpListener::from_std(listener).unwrap();
    let mut next = 0;
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        if is_symbols_request(&stream).await {
            tokio::spawn(answer_symbols(stream, symbols.clone()));
            continue;
        }
        let script = scripts
            .get(next)
            .or(scripts.last())
            .cloned()
            .unwrap_or_else(|| Script::answering(&[]));
        tokio::spawn(serve_connection(stream, next, script, log.clone()));
        next += 1;
    }
}

async fn is_symbols_request(stream: &TcpStream) -> bool {
    let mut head = [0u8; 15];
    loop {
        let peeked = stream.peek(&mut head).await.unwrap();
        if peeked == 0 {
            return false;
        }
        if peeked >= head.len() {
            return head == SYMBOLS_REQUEST;
        }
    }
}

async fn answer_symbols(mut stream: TcpStream, symbols: Option<String>) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 512];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let response = match symbols {
        Some(body) => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        ),
        None => {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        }
    };
    stream.write_all(response.as_bytes()).await.unwrap();
}

struct Handshake {
    log: Arc<Log>,
    refuse: bool,
}

impl Callback for Handshake {
    #[allow(
        clippy::result_large_err,
        reason = "tungstenite fixes the Callback error type"
    )]
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let header = request
            .headers()
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        self.log.auth.lock().unwrap().push(header);
        if self.refuse {
            let mut rejection = ErrorResponse::new(Some("unauthorized".to_string()));
            *rejection.status_mut() = StatusCode::UNAUTHORIZED;
            return Err(rejection);
        }
        Ok(response)
    }
}

async fn serve_connection(stream: TcpStream, index: usize, script: Script, log: Arc<Log>) {
    let handshake = accept_hdr_async(
        stream,
        Handshake {
            log: log.clone(),
            refuse: script.refuse,
        },
    )
    .await;
    if let Ok(mut socket) = handshake {
        if script.greeting {
            socket
                .send(Message::Text(HEARTBEAT_GREETING.to_string()))
                .await
                .unwrap();
        }
        let mut orders = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            log.texts.lock().unwrap().push((index, text.clone()));
            if text.starts_with("{\"H\"") {
                continue;
            }
            if let Some(frames) = script.replies.get(orders) {
                for frame in frames {
                    socket.send(Message::Text(frame.clone())).await.unwrap();
                }
            }
            if let Some(responder) = &script.responder {
                let is_answering = responder.answer_limit.is_none_or(|limit| orders < limit);
                if is_answering {
                    tokio::time::sleep(responder.delay).await;
                    for frame in responder.frames(index, orders) {
                        socket.send(Message::Text(frame)).await.unwrap();
                    }
                }
            }
            orders += 1;
        }
    }
    *log.closed.lock().unwrap() += 1;
    log.changed.notify_all();
}
