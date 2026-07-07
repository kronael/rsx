//! Protocol comparison benchmark — uniform harness.
//!
//! All protocols implement `EchoClient`: one method, `ping()`.
//! The benchmark harness is identical for all of them:
//!
//! Client (Criterion timer) thread pinned to core 2. Server echo
//! threads (where applicable: raw UDP, KCP) pinned to core 3.
//! Quinn + TCP run on single-threaded current_thread Tokio runtimes,
//! so server tasks share the client's core (no extra OS thread).
//!
//!   fn run_bench(c, name, client) {
//!       b.iter(|| client.ping(&payload, &mut buf));
//!   }
//!
//! Server setup, async runtimes, and framing are internal to each
//! impl. The bench only sees: send 64 bytes, receive echo, measure RTT.
//!
//! Protocols (all measure a 128-byte payload to match cmp_rtt_bench):
//!   raw_udp          — baseline: sendto + recvfrom, no framing
//!   kcp_spin         — KCP turbo: flush() immediately, spin echo
//!   quinn_persistent — QUIC persistent stream + 4B length framing
//!   tcp_nodelay      — TCP nodelay, persistent connection (read_exact)
//!
//! See compare/*.md for protocol analysis. Run all:
//!
//!   cargo bench -p rsx-cast --bench compare_all

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;

fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let c = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let s = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (c, s)
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Minimal interface for a loopback echo client.
/// Send `payload`, block until echo received, write into `buf`, return length.
/// All setup (server thread, runtime, TLS) happens in the constructor.
trait EchoClient {
    fn ping(&mut self, payload: &[u8], buf: &mut [u8]) -> usize;
}

// 128 B = size_of::<FillRecord>() per rsx-messages/src/lib.rs:78, so
// every protocol here measures the same payload size as cmp_rtt_bench.
const PAYLOAD_LEN: usize = 128;

fn run_bench(c: &mut Criterion, name: &str, client: &mut dyn EchoClient) {
    let payload = [0xAAu8; PAYLOAD_LEN];
    let mut buf = [0u8; 256];
    c.bench_function(name, |b| {
        b.iter(|| {
            let n = client.ping(black_box(&payload), &mut buf);
            black_box(n);
        });
    });
}

// ── Raw UDP ───────────────────────────────────────────────────────────────────

use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;

struct RawUdpClient {
    sock: UdpSocket,
    srv_addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for RawUdpClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Send a wake-up packet so the (nonblocking) server thread
        // gets one final recv and notices the stop flag.
        let _ = self.sock.send_to(&[0xFFu8; 1], self.srv_addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl RawUdpClient {
    fn new() -> Self {
        let (_, srv_core) = pick_cores();
        let srv = UdpSocket::bind("127.0.0.1:0").unwrap();
        srv.set_nonblocking(true).unwrap();
        let srv_addr = srv.local_addr().unwrap();

        let cli = UdpSocket::bind("127.0.0.1:0").unwrap();
        cli.set_nonblocking(true).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            core_affinity::set_for_current(srv_core);
            let mut buf = [0u8; 256];
            while !stop2.load(Ordering::Relaxed) {
                match srv.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        if n >= 1 && buf[0] == 0xFF { return; }
                        let _ = srv.send_to(&buf[..n], src);
                    }
                    Err(_) => std::hint::spin_loop(),
                }
            }
        });

        Self { sock: cli, srv_addr, stop, handle: Some(handle) }
    }
}

impl EchoClient for RawUdpClient {
    fn ping(&mut self, payload: &[u8], buf: &mut [u8]) -> usize {
        self.sock.send_to(payload, self.srv_addr).unwrap();
        loop {
            match self.sock.recv_from(buf) {
                Ok((n, _)) => return n,
                Err(_) => std::hint::spin_loop(),
            }
        }
    }
}

// ── KCP spin ──────────────────────────────────────────────────────────────────

use kcp::Kcp;
use std::collections::VecDeque;
use std::io;
use std::sync::Mutex;
use std::time::Instant;

type OutQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

struct UdpOutput { queue: OutQueue }

impl io::Write for UdpOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.queue.lock().unwrap().push_back(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn make_kcp(conv: u32, queue: OutQueue) -> Kcp<UdpOutput> {
    let mut k = Kcp::new(conv, UdpOutput { queue });
    k.set_nodelay(true, 1, 2, true); // turbo: nodelay, interval=1ms, resend=2, nc
    k.set_wndsize(128, 128);
    k.set_mtu(1400).unwrap();
    // kcp requires one update() to set its `updated` flag before the
    // first flush(), else flush() returns Err(NeedUpdate). Loopback has
    // no loss so the retransmit clock is irrelevant; prime once at 0.
    let _ = k.update(0);
    k
}

fn drain(q: &OutQueue, sock: &UdpSocket, dest: std::net::SocketAddr) {
    let mut lock = q.lock().unwrap();
    while let Some(pkt) = lock.pop_front() { let _ = sock.send_to(&pkt, dest); }
}

struct KcpSpinClient {
    kcp: Kcp<UdpOutput>,
    sock: UdpSocket,
    srv_addr: std::net::SocketAddr,
    out: OutQueue,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for KcpSpinClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl KcpSpinClient {
    fn new() -> Self {
        let (_, srv_core) = pick_cores();
        let srv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        srv_sock.set_nonblocking(true).unwrap();
        let srv_addr = srv_sock.local_addr().unwrap();

        let cli_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        cli_sock.set_nonblocking(true).unwrap();
        let cli_addr = cli_sock.local_addr().unwrap();

        let srv_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));
        let srv_out2 = Arc::clone(&srv_out);
        let cli_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));

        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);

        let srv_sock2 = srv_sock.try_clone().unwrap();
        let handle = thread::spawn(move || {
            core_affinity::set_for_current(srv_core);
            let mut kcp = make_kcp(1, srv_out2);
            let mut buf = [0u8; 2048];
            let mut msg = [0u8; 2048];
            while !stop2.load(Ordering::Relaxed) {
                let mut got = false;
                while let Ok((n, _)) = srv_sock2.recv_from(&mut buf) {
                    let _ = kcp.input(&buf[..n]);
                    got = true;
                }
                if got {
                    while let Ok(n) = kcp.recv(&mut msg) { let _ = kcp.send(&msg[..n]); }
                    kcp.flush().unwrap();
                    drain(&srv_out, &srv_sock2, cli_addr);
                } else {
                    std::hint::spin_loop();
                }
            }
        });

        let mut kcp = make_kcp(1, Arc::clone(&cli_out));

        // Warmup with the actual measured payload size.
        kcp.send(&[0x42u8; PAYLOAD_LEN]).unwrap();
        kcp.flush().unwrap();
        drain(&cli_out, &cli_sock, srv_addr);
        let deadline = Instant::now() + std::time::Duration::from_millis(200);
        let mut wbuf = [0u8; 2048];
        let mut mbuf = [0u8; 2048];
        loop {
            if Instant::now() > deadline { break; }
            while let Ok((n, _)) = cli_sock.recv_from(&mut wbuf) { let _ = kcp.input(&wbuf[..n]); }
            if kcp.recv(&mut mbuf).is_ok() { break; }
            std::hint::spin_loop();
        }

        Self {
            kcp,
            sock: cli_sock,
            srv_addr,
            out: cli_out,
            stop,
            handle: Some(handle),
        }
    }
}

impl EchoClient for KcpSpinClient {
    fn ping(&mut self, payload: &[u8], buf: &mut [u8]) -> usize {
        self.kcp.send(payload).unwrap();
        // flush() bypasses the update() timer: DATA frame sent immediately.
        self.kcp.flush().unwrap();
        drain(&self.out, &self.sock, self.srv_addr);
        let deadline = Instant::now() + std::time::Duration::from_millis(100);
        loop {
            if Instant::now() > deadline { return 0; }
            while let Ok((n, _)) = self.sock.recv_from(buf) {
                let _ = self.kcp.input(&buf[..n]);
            }
            // Return the actual decoded length so payload-size
            // changes (e.g. 64 → 128) don't silently mis-measure.
            if let Ok(n) = self.kcp.recv(buf) { return n; }
            std::hint::spin_loop();
        }
    }
}

// ── Quinn persistent stream ───────────────────────────────────────────────────

use quinn::Endpoint;
use quinn::RecvStream;
use quinn::SendStream;
use quinn::ServerConfig;
use quinn::ClientConfig;
use rustls::pki_types::PrivatePkcs8KeyDer;
use tokio::runtime::Builder;

async fn read_framed(recv: &mut RecvStream, buf: &mut [u8]) -> usize {
    let mut len_buf = [0u8; 4];
    if recv.read_exact(&mut len_buf).await.is_err() { return 0; }
    let n = u32::from_le_bytes(len_buf) as usize;
    if recv.read_exact(&mut buf[..n]).await.is_err() { return 0; }
    n
}

async fn write_framed(send: &mut SendStream, data: &[u8]) {
    let len = (data.len() as u32).to_le_bytes();
    let _ = send.write_all(&len).await;
    let _ = send.write_all(data).await;
}

struct QuinnPersistentClient {
    rt: tokio::runtime::Runtime,
    send: SendStream,
    recv: RecvStream,
    _server_ep: Endpoint,
    _client_ep: Endpoint,
    _conn: quinn::Connection,
}

impl QuinnPersistentClient {
    fn new() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        let (server_ep, client_ep, cli_conn, srv_conn) = rt.block_on(async {
            let cert =
                rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der = cert.cert.der().clone();
            let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

            let srv_cfg = ServerConfig::with_single_cert(
                vec![cert_der.clone()], key_der.into(),
            ).unwrap();

            let server_ep = Endpoint::server(
                srv_cfg, "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap(),
            ).unwrap();
            let srv_addr = server_ep.local_addr().unwrap();

            // Accept exactly one connection.
            let server_ep2 = server_ep.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                if let Some(inc) = server_ep2.accept().await {
                    if let Ok(conn) = inc.await { let _ = tx.send(conn); }
                }
            });

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der).unwrap();
            let cli_cfg = ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
            let mut client_ep = Endpoint::client(
                "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap(),
            ).unwrap();
            client_ep.set_default_client_config(cli_cfg);
            let cli_conn = client_ep.connect(srv_addr, "localhost").unwrap().await.unwrap();

            // Wait for server to accept.
            let srv_conn = rx.await.expect("server accept");
            (server_ep, client_ep, cli_conn, srv_conn)
        });

        // Open one persistent bidirectional stream.
        let (send, recv, srv_send, srv_recv) = rt.block_on(async {
            let (mut cli_send, cli_recv) = cli_conn.open_bi().await.unwrap();
            // QUIC opens a bi stream lazily: open_bi() puts nothing on the
            // wire until the first write, so the server's accept_bi() never
            // resolves. Write one priming byte to flush the stream open.
            cli_send.write_all(&[0u8]).await.unwrap();
            let (srv_send, mut srv_recv) = srv_conn.accept_bi().await.unwrap();
            // Discard the priming byte so the length-framed echo lines up.
            let mut prime = [0u8; 1];
            srv_recv.read_exact(&mut prime).await.unwrap();
            (cli_send, cli_recv, srv_send, srv_recv)
        });

        // Server echo loop.
        rt.spawn(async move {
            let mut s = srv_send;
            let mut r = srv_recv;
            let mut buf = [0u8; 256];
            loop {
                let n = read_framed(&mut r, &mut buf).await;
                if n == 0 { break; }
                write_framed(&mut s, &buf[..n]).await;
            }
        });

        Self {
            rt,
            send,
            recv,
            _server_ep: server_ep,
            _client_ep: client_ep,
            _conn: cli_conn,
        }
    }
}

impl EchoClient for QuinnPersistentClient {
    fn ping(&mut self, payload: &[u8], buf: &mut [u8]) -> usize {
        self.rt.block_on(async {
            write_framed(&mut self.send, payload).await;
            read_framed(&mut self.recv, buf).await
        })
    }
}

// ── TCP nodelay ───────────────────────────────────────────────────────────────

use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

struct TcpNodelay {
    rt: tokio::runtime::Runtime,
    stream: TcpStream,
}

impl TcpNodelay {
    fn new() -> Self {
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        let stream = rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                while let Ok((mut sock, _)) = listener.accept().await {
                    sock.set_nodelay(true).unwrap();
                    tokio::spawn(async move {
                        // Read exactly PAYLOAD_LEN per iter and echo it.
                        // TCP is a byte stream — a single read() can return
                        // any partial length, so we read_exact for parity
                        // with the framed PAYLOAD_LEN sends.
                        let mut buf = [0u8; PAYLOAD_LEN];
                        loop {
                            if sock.read_exact(&mut buf).await.is_err() { break; }
                            if sock.write_all(&buf).await.is_err() { break; }
                        }
                    });
                }
            });
            let stream = TcpStream::connect(addr).await.unwrap();
            stream.set_nodelay(true).unwrap();
            stream
        });
        Self { rt, stream }
    }
}

impl EchoClient for TcpNodelay {
    fn ping(&mut self, payload: &[u8], buf: &mut [u8]) -> usize {
        self.rt.block_on(async {
            self.stream.write_all(payload).await.unwrap();
            // read_exact: TCP is a byte stream, a single read() can
            // return a short prefix, leaving trailing bytes for the
            // next iter and under-measuring RTT.
            self.stream.read_exact(&mut buf[..PAYLOAD_LEN]).await.unwrap();
            PAYLOAD_LEN
        })
    }
}

// ── Harness ───────────────────────────────────────────────────────────────────

fn bench_all(c: &mut Criterion) {
    let (cli_core, _) = pick_cores();
    core_affinity::set_for_current(cli_core);
    run_bench(c, "raw_udp_128b",            &mut RawUdpClient::new());
    run_bench(c, "kcp_spin_flush_128b",     &mut KcpSpinClient::new());
    run_bench(c, "quinn_persistent_128b",   &mut QuinnPersistentClient::new());
    run_bench(c, "tcp_nodelay_128b",        &mut TcpNodelay::new());
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_all
}
criterion_main!(benches);
