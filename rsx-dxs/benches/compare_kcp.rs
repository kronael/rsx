//! KCP loopback round-trip comparison.
//!
//! What this measures
//! -----------------
//! KCP in "turbo mode" (nodelay=1, interval=1ms, resend=2, nc=1)
//! vs raw UDP (via udp_rtt_bench) on the same 127.0.0.1 loopback.
//! Payload: 64 bytes (one cache line; matches CMP exchange frame).
//!
//! KCP is an ACK-based ARQ protocol (not NAK-based). Loss detection
//! is at the sender — retransmit fires after `resend` out-of-order
//! ACKs. Fast retransmit path is ~1.5–2 RTT vs ~1 RTT for NAK.
//! The `interval` parameter (minimum flush tick) is the dominant
//! latency term on zero-loss loopback: with interval=1ms, the
//! floor is ~1 ms regardless of syscall cost.
//!
//! Compare with:
//!   udp_rtt_bench    raw UDP floor (~2 µs)
//!   cmp_rtt_bench    CMP NAK overhead (~10 µs)
//!   compare_quinn    QUIC overhead (~200–500 µs)
//!
//! See compare/kcp.md for protocol details and design comparison.
//!
//! Caveats
//! -------
//! - Zero packet loss. KCP's advantage over raw UDP emerges under
//!   loss. Run with `sudo tc qdisc add dev lo root netem loss 0.1%`
//!   to observe loss-recovery behaviour.
//! - KCP timer is driven by `std::thread::sleep(1ms)` in the
//!   echoer thread. This is realistic for the KCP update() model
//!   but means OS scheduler jitter (±0.5ms) dominates the reading.

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
use std::thread;
use std::time::Duration;
use std::time::Instant;

fn now_ms() -> u32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_millis() as u32
}

/// Shared queue from KCP output callback → UDP sender thread.
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

fn make_kcp(queue: OutQueue) -> Kcp<UdpOutput> {
    let mut kcp = Kcp::new(0, UdpOutput { queue });
    // Turbo mode: nodelay=true, interval=1ms, resend=2, nc=true
    kcp.set_nodelay(true, 1, 2, true);
    kcp.set_wndsize(128, 128);
    kcp.set_mtu(1400).unwrap();
    kcp
}

fn bench_kcp_rtt(c: &mut Criterion) {
    // --- server side ---
    let srv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    srv_sock.set_nonblocking(true).unwrap();
    let srv_addr = srv_sock.local_addr().unwrap();

    // --- client side ---
    let cli_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    cli_sock.set_nonblocking(true).unwrap();
    let _cli_addr = cli_sock.local_addr().unwrap();

    let srv_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));
    let cli_out: OutQueue = Arc::new(Mutex::new(VecDeque::new()));

    let srv_out2 = Arc::clone(&srv_out);
    let cli_out2 = Arc::clone(&cli_out);

    // Server thread: echo loop driven by 1ms timer.
    let srv_sock2 = srv_sock.try_clone().unwrap();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);

    thread::spawn(move || {
        let mut kcp = make_kcp(srv_out2);
        let mut buf = [0u8; 2048];
        let mut msg = [0u8; 2048];
        let mut peer = None::<std::net::SocketAddr>;

        while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
            // Drain UDP → KCP input
            loop {
                match srv_sock2.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        peer = Some(src);
                        let _ = kcp.input(&buf[..n]);
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock =>
                    {
                        break;
                    }
                    Err(_) => return,
                }
            }
            // Drain KCP recv → echo back
            if let Some(addr) = peer {
                while let Ok(n) = kcp.recv(&mut msg) {
                    let _ = kcp.send(&msg[..n]);
                }
                kcp.update(now_ms()).unwrap();
                // Flush KCP output queue → UDP
                let mut q = srv_out.lock().unwrap();
                while let Some(pkt) = q.pop_front() {
                    let _ = srv_sock2.send_to(&pkt, addr);
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    });

    let mut kcp = make_kcp(Arc::clone(&cli_out));
    let mut recv_buf = [0u8; 2048];
    let mut udp_buf = [0u8; 2048];

    // Warm up: establish KCP state before measuring.
    {
        let payload = [0x42u8; 64];
        kcp.send(&payload).unwrap();
        kcp.update(now_ms()).unwrap();
        {
            let mut q = cli_out2.lock().unwrap();
            while let Some(pkt) = q.pop_front() {
                let _ = cli_sock.send_to(&pkt, srv_addr);
            }
        }
        let deadline = Instant::now() + Duration::from_millis(200);
        loop {
            if Instant::now() > deadline {
                break;
            }
            loop {
                match cli_sock.recv_from(&mut udp_buf) {
                    Ok((n, _)) => {
                        let _ = kcp.input(&udp_buf[..n]);
                    }
                    Err(_) => break,
                }
            }
            if kcp.recv(&mut recv_buf).is_ok() {
                break;
            }
            kcp.update(now_ms()).unwrap();
            {
                let mut q = cli_out2.lock().unwrap();
                while let Some(pkt) = q.pop_front() {
                    let _ = cli_sock.send_to(&pkt, srv_addr);
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    let payload = [0xAAu8; 64];

    c.bench_function("kcp_rtt_loopback_64b_turbo", |b| {
        b.iter(|| {
            kcp.send(black_box(&payload)).unwrap();
            kcp.update(now_ms()).unwrap();
            {
                let mut q = cli_out2.lock().unwrap();
                while let Some(pkt) = q.pop_front() {
                    let _ = cli_sock.send_to(&pkt, srv_addr);
                }
            }
            // Spin until KCP delivers the echoed message.
            let deadline = Instant::now() + Duration::from_millis(500);
            loop {
                if Instant::now() > deadline {
                    break;
                }
                loop {
                    match cli_sock.recv_from(&mut udp_buf) {
                        Ok((n, _)) => {
                            let _ = kcp.input(&udp_buf[..n]);
                        }
                        Err(_) => break,
                    }
                }
                if kcp.recv(&mut recv_buf).is_ok() {
                    black_box(&recv_buf);
                    break;
                }
                kcp.update(now_ms()).unwrap();
                {
                    let mut q = cli_out2.lock().unwrap();
                    while let Some(pkt) = q.pop_front() {
                        let _ = cli_sock.send_to(&pkt, srv_addr);
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
    });

    stop.store(true, std::sync::atomic::Ordering::Release);
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_kcp_rtt
}
criterion_main!(benches);
