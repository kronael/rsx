//! TCP loopback round-trip baseline.
//!
//! What this measures
//! -----------------
//! `std::net::TcpListener` / `TcpStream` on 127.0.0.1, one
//! persistent connection, `TCP_NODELAY` on both ends. Both
//! sockets non-blocking, both threads busy-spinning on
//! `read` / `write` — matches the style of `udp_rtt_bench.rs`
//! and the spin variant of `compare_kcp.rs`. 64-byte payload
//! (one cache line).
//!
//! The 3-way handshake runs once in setup. The timed loop
//! reuses the same connection for every iteration — same
//! convention as the QUIC bench. This is the best-case TCP
//! latency a long-lived consumer (like DXS WAL replay) sees
//! once its session is open. It is NOT the cost of opening
//! a new TCP connection per message.
//!
//! TCP is a byte stream, not message-oriented: a single
//! `write_all(64)` can split across multiple `read()`s. The
//! receiver drains exactly 64 bytes per iteration via a
//! `read_exact`-style spin loop. A naive single `read()`
//! would race and undercount.
//!
//! Why both nodelay + nonblocking + spin
//! ------------------------------------
//! - `TCP_NODELAY` disables Nagle's algorithm (RFC 896). Without
//!   it the bench measures the Nagle ↔ delayed-ACK pathological
//!   interaction (~40 ms stalls), not TCP itself.
//! - Non-blocking + spin keeps both threads cache-hot and avoids
//!   reactor wake-up between syscalls. Same shape as the UDP
//!   bench so the numbers are directly comparable.
//!
//! Assumptions / caveats
//! --------------------
//! - Loopback, not real net (no PHY/MAC, no driver, no NIC IRQ).
//! - Single persistent connection — no handshake in timed loop.
//! - Spinning + nodelay is the floor; production async TCP
//!   (see `compare_quinn.rs::tcp_rtt_nodelay`) is ~10–100×
//!   slower because of the tokio reactor.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

const PAYLOAD: usize = 64;

/// Read exactly `buf.len()` bytes from a non-blocking TCP stream
/// by spinning on `WouldBlock`. TCP is a stream — one `read`
/// can return any 1..=PAYLOAD bytes — so we loop until full.
/// Returns false if the peer closed.
fn read_exact_spin(sock: &mut TcpStream, buf: &mut [u8]) -> bool {
    let mut filled = 0;
    while filled < buf.len() {
        match sock.read(&mut buf[filled..]) {
            Ok(0) => return false,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::hint::spin_loop();
            }
            Err(_) => return false,
        }
    }
    true
}

/// Write exactly `buf.len()` bytes by spinning on `WouldBlock`.
/// Returns false on peer close / error.
fn write_all_spin(sock: &mut TcpStream, buf: &[u8]) -> bool {
    let mut sent = 0;
    while sent < buf.len() {
        match sock.write(&buf[sent..]) {
            Ok(0) => return false,
            Ok(n) => sent += n,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::hint::spin_loop();
            }
            Err(_) => return false,
        }
    }
    true
}

fn bench_tcp_rtt_loopback(c: &mut Criterion) {
    // Establish the listener + connection once. The 3-way
    // handshake cost is OUT of the timed loop.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let listener_addr = listener.local_addr().unwrap();

    // Echoer thread: accept one connection, then spin-echo
    // until the client closes.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_nodelay(true).unwrap();
        sock.set_nonblocking(true).unwrap();

        let mut buf = [0u8; PAYLOAD];
        while !stop_clone.load(Ordering::Relaxed) {
            if !read_exact_spin(&mut sock, &mut buf) {
                return;
            }
            if !write_all_spin(&mut sock, &buf) {
                return;
            }
        }
    });

    let mut pinger = TcpStream::connect(listener_addr).unwrap();
    pinger.set_nodelay(true).unwrap();
    pinger.set_nonblocking(true).unwrap();

    let payload = [0xAAu8; PAYLOAD];
    let mut recv_buf = [0u8; PAYLOAD];

    c.bench_function("tcp_rtt_loopback_64b", |b| {
        b.iter(|| {
            // Panic on EOF / non-WouldBlock error — if the echo
            // thread dies mid-bench, Criterion would otherwise
            // record meaningless near-zero timings as the
            // failing recv returns immediately.
            assert!(write_all_spin(&mut pinger, black_box(&payload)));
            assert!(read_exact_spin(&mut pinger, &mut recv_buf));
            black_box(&recv_buf);
        });
    });

    // Signal exit, drop the client so echoer's read returns Err.
    stop.store(true, Ordering::Release);
    drop(pinger);
    let _ = handle.join();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_tcp_rtt_loopback
}
criterion_main!(benches);
