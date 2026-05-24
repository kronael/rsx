//! KCP loopback round-trip comparison.
//!
//! Two benchmarks:
//!
//! `kcp_rtt_naive` — timer-driven update() every 1ms (sleep-based).
//!   Shows the latency floor imposed by the polling model: ~1–15 ms.
//!   This is how most KCP integrations work in practice.
//!
//! `kcp_rtt_spin` — optimal: no sleep, spin-poll, explicit flush()
//!   after every send(). Server busy-spins on recv_from, calls
//!   kcp.input() + kcp.recv() + kcp.send() + kcp.flush() immediately.
//!   Bypasses the update() timer entirely. Shows KCP's true protocol
//!   overhead: 24B header + ACK bookkeeping, no artificial floor.
//!   Expected: ~20–100 µs — dominated by two sendto syscalls + ACK
//!   round-trip. Contrast: CMP NAK RTT ~10 µs (one sendto + one
//!   recvfrom, receiver sends NAK only on loss).
//!
//! Protocol difference: KCP is ACK-based. Even with spin+flush, the
//! server must process an ACK from the client for every echo it sends.
//! CMP is NAK-based — no per-message ACK, only a periodic StatusMessage
//! (every 10ms). On zero-loss loopback this means CMP has ~half the
//! control-plane traffic of KCP.
//!
//! See compare/kcp.md for full protocol analysis.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use kcp::Kcp;
use std::collections::VecDeque;
use std::io;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

fn now_ms() -> u32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_millis() as u32
}

type OutQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

struct UdpOutput {
    queue: OutQueue,
}

impl io::Write for UdpOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.queue.lock().unwrap().push_back(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn make_kcp(conv: u32, queue: OutQueue) -> Kcp<UdpOutput> {
    let mut kcp = Kcp::new(conv, UdpOutput { queue });
    // nodelay=true, interval=1ms, resend=2 (fast retransmit), nc=true (no CWND)
    kcp.set_nodelay(true, 1, 2, true);
    kcp.set_wndsize(128, 128);
    kcp.set_mtu(1400).unwrap();
    kcp
}

fn drain_output(queue: &OutQueue, sock: &UdpSocket, dest: std::net::SocketAddr) {
    let mut q = queue.lock().unwrap();
    while let Some(pkt) = q.pop_front() {
        let _ = sock.send_to(&pkt, dest);
    }
}

// ── naive: sleep-based update() ──────────────────────────────────────────────

fn bench_kcp_naive(c: &mut Criterion) {
    let srv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    srv_sock.set_nonblocking(true).unwrap();
    let srv_addr = srv_sock.local_addr().unwrap();

    let cli_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    cli_sock.set_nonblocking(true).unwrap();
    let cli_addr = cli_sock.local_addr().unwrap();

    let srv_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));
    let srv_out2 = Arc::clone(&srv_out);

    let cli_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));
    let cli_out2 = Arc::clone(&cli_out);

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);

    let srv_sock2 = srv_sock.try_clone().unwrap();
    thread::spawn(move || {
        let mut kcp = make_kcp(1, srv_out2);
        let mut buf = [0u8; 2048];
        let mut msg = [0u8; 2048];
        while !stop2.load(Ordering::Relaxed) {
            loop {
                match srv_sock2.recv_from(&mut buf) {
                    Ok((n, _)) => { let _ = kcp.input(&buf[..n]); }
                    Err(_) => break,
                }
            }
            while let Ok(n) = kcp.recv(&mut msg) {
                let _ = kcp.send(&msg[..n]);
            }
            kcp.update(now_ms()).unwrap();
            drain_output(&srv_out, &srv_sock2, cli_addr);
            thread::sleep(Duration::from_millis(1));
        }
    });

    let mut kcp = make_kcp(1, Arc::clone(&cli_out));
    let mut udp_buf = [0u8; 2048];
    let mut msg_buf = [0u8; 2048];

    // Warmup.
    kcp.send(&[0x42u8; 64]).unwrap();
    kcp.update(now_ms()).unwrap();
    drain_output(&cli_out2, &cli_sock, srv_addr);
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        if Instant::now() > deadline { break; }
        while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) { let _ = kcp.input(&udp_buf[..n]); }
        if kcp.recv(&mut msg_buf).is_ok() { break; }
        kcp.update(now_ms()).unwrap();
        drain_output(&cli_out2, &cli_sock, srv_addr);
        thread::sleep(Duration::from_millis(1));
    }

    let payload = [0xAAu8; 64];
    c.bench_function("kcp_rtt_naive_1ms_interval", |b| {
        b.iter(|| {
            kcp.send(black_box(&payload)).unwrap();
            kcp.update(now_ms()).unwrap();
            drain_output(&cli_out2, &cli_sock, srv_addr);
            let deadline = Instant::now() + Duration::from_millis(500);
            loop {
                if Instant::now() > deadline { break; }
                while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) { let _ = kcp.input(&udp_buf[..n]); }
                if kcp.recv(&mut msg_buf).is_ok() { black_box(&msg_buf); break; }
                kcp.update(now_ms()).unwrap();
                drain_output(&cli_out2, &cli_sock, srv_addr);
                thread::sleep(Duration::from_millis(1));
            }
        });
    });

    stop.store(true, Ordering::Release);
}

// ── spin: flush() immediately, no sleep ──────────────────────────────────────

fn bench_kcp_spin(c: &mut Criterion) {
    let srv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    srv_sock.set_nonblocking(true).unwrap();
    let srv_addr = srv_sock.local_addr().unwrap();

    let cli_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    cli_sock.set_nonblocking(true).unwrap();
    let cli_addr = cli_sock.local_addr().unwrap();

    let srv_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));
    let srv_out2 = Arc::clone(&srv_out);

    let cli_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));
    let cli_out2 = Arc::clone(&cli_out);

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);

    let srv_sock2 = srv_sock.try_clone().unwrap();
    thread::spawn(move || {
        let mut kcp = make_kcp(2, srv_out2);
        let mut buf = [0u8; 2048];
        let mut msg = [0u8; 2048];
        // Spin: no sleep. Call update() + flush() on every loop iteration.
        while !stop2.load(Ordering::Relaxed) {
            let mut got = false;
            while let Ok((n, _)) = srv_sock2.recv_from(&mut buf) {
                let _ = kcp.input(&buf[..n]);
                got = true;
            }
            if got {
                while let Ok(n) = kcp.recv(&mut msg) {
                    let _ = kcp.send(&msg[..n]);
                }
                // flush() bypasses the update() timer — sends immediately.
                kcp.flush().unwrap();
                drain_output(&srv_out, &srv_sock2, cli_addr);
            } else {
                std::hint::spin_loop();
            }
        }
    });

    let mut kcp = make_kcp(2, Arc::clone(&cli_out));
    let mut udp_buf = [0u8; 2048];
    let mut msg_buf = [0u8; 2048];

    // Warmup.
    kcp.send(&[0x42u8; 64]).unwrap();
    kcp.flush().unwrap();
    drain_output(&cli_out2, &cli_sock, srv_addr);
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        if Instant::now() > deadline { break; }
        while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) { let _ = kcp.input(&udp_buf[..n]); }
        if kcp.recv(&mut msg_buf).is_ok() { break; }
        std::hint::spin_loop();
    }

    let payload = [0xAAu8; 64];
    c.bench_function("kcp_rtt_spin_flush", |b| {
        b.iter(|| {
            kcp.send(black_box(&payload)).unwrap();
            // flush() sends the DATA frame immediately (no timer wait).
            kcp.flush().unwrap();
            drain_output(&cli_out2, &cli_sock, srv_addr);
            // Spin until the echo DATA arrives.
            let deadline = Instant::now() + Duration::from_millis(100);
            loop {
                if Instant::now() > deadline { break; }
                while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) {
                    let _ = kcp.input(&udp_buf[..n]);
                }
                if kcp.recv(&mut msg_buf).is_ok() {
                    black_box(&msg_buf);
                    break;
                }
                std::hint::spin_loop();
            }
        });
    });

    stop.store(true, Ordering::Release);
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_kcp_naive, bench_kcp_spin
}
criterion_main!(benches);
