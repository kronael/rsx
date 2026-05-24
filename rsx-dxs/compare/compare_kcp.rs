//! KCP loopback round-trip — `compare/kcp.md` companion bench.
//!
//! What this measures
//! ------------------
//! Application-visible loopback RTT for KCP turbo mode, on the same
//! host, payload size matched to `cmp_rtt_bench.rs` (128 B —
//! `FillRecord` is `mem::size_of::<FillRecord>() == 128`).
//! Two variants:
//!
//! `kcp_rtt_naive_1ms_interval_128b` — timer-driven `update()` every
//!   1 ms on both client and server. Sender sleeps for 1 ms between
//!   poll cycles. This is the cadence most KCP integrations use in
//!   practice (mirrors a Tokio interval / select timer). Expected:
//!   tens of milliseconds RTT, dominated by `sleep(1ms)` polling
//!   on both sides + KCP scheduler granularity. NOT comparable
//!   to CMP's RTT; this is a "realistic integration mode"
//!   datapoint, not the headline.
//!
//! `kcp_rtt_spin_flush_128b` — busy-spin server, explicit `flush()`
//!   after every `send()` on both sides. This bypasses the
//!   `update()` scheduler: DATA frames AND ACK frames are emitted
//!   immediately. Expected: KCP's true protocol overhead — two
//!   `sendto` per RTT for DATA + standalone ACK, plus the KCP 24 B
//!   header parsing + ACK-list bookkeeping. Closer to apples-to-
//!   apples with CMP's `cmp_rtt_fill_echo` (no per-frame ACK on
//!   CMP — see `compare/kcp.md` for the asymmetry).
//!
//! What is NOT measured
//! --------------------
//! - Loss recovery. To bench under loss:
//!     sudo tc qdisc add dev lo root netem loss 0.1%
//!     cargo bench -p rsx-dxs --bench compare_kcp
//!     sudo tc qdisc del dev lo root
//!   The bench itself does not require root or tc.
//! - WAN behaviour (loopback only).
//! - Multi-stream / fan-out (KCP is single-stream per `conv`).
//! - Memory / CPU under sustained load (single-iter RTT only).
//!
//! Apples-to-apples with `cmp_rtt_bench.rs`
//! ----------------------------------------
//! Same payload size (128 B). Same Criterion sample_size (50; CMP
//! uses Criterion's default 100, this bench uses 50 for setup-time
//! tractability of the naive variant). Same loopback (127.0.0.1).
//! Spin-loop client polling. No TLS, no handshake.
//! Client (Criterion timer) pinned to core 2, server echo thread
//! to core 3. The naive variant sleeps 1 ms; pinning still
//! reduces tail variance there.
//!
//! Adapter-overhead caveat
//! -----------------------
//! KCP's `io::Write` adapter pushes outbound frames into a
//! per-thread `RefCell<VecDeque<Vec<u8>>>` (no Mutex — the Kcp
//! instance and the drain are both touched only by the owning
//! thread), then `drain_output()` pops and `sendto`s. The
//! `Vec<u8>::from(buf)` allocation per frame is the dominant
//! adapter cost (~50–100 ns). CMP's `CmpSender::send` writes
//! directly to a pre-allocated ring slot (no alloc on send). This
//! adapter cost is fundamental to using the `kcp` crate's
//! callback-style API and not easy to eliminate without forking
//! the crate. Documented; not removed.
//!
//! Bootstrap note
//! --------------
//! The Rust `kcp` crate requires at least one `update()` call
//! before `flush()` works (otherwise `flush()` returns
//! `Error::NeedUpdate`). Both sides pay this once at startup;
//! after that, `flush()` calls in the hot loop bypass the
//! scheduler.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use kcp::Kcp;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

// Payload size matched to FillRecord (rsx-messages/src/lib.rs:78
// asserts size_of::<FillRecord>() == 128). cmp_rtt_bench.rs sends
// one FillRecord per iter; this bench sends 128 B of payload bytes.
const PAYLOAD_LEN: usize = 128;

fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let c = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let s = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (c, s)
}

fn now_ms() -> u32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_millis() as u32
}

type OutQueue = Rc<RefCell<VecDeque<Vec<u8>>>>;

struct UdpOutput {
    queue: OutQueue,
}

impl io::Write for UdpOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.queue.borrow_mut().push_back(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn make_kcp(conv: u32, queue: OutQueue) -> Kcp<UdpOutput> {
    let mut kcp = Kcp::new(conv, UdpOutput { queue });
    // turbo: nodelay=1, interval=1ms, resend=2 (fast retransmit), nc=1 (no CWND)
    kcp.set_nodelay(true, 1, 2, true);
    kcp.set_wndsize(128, 128);
    kcp.set_mtu(1400).expect("set_mtu(1400) ok");
    kcp
}

fn drain_output(queue: &OutQueue, sock: &UdpSocket, dest: SocketAddr) {
    let mut q = queue.borrow_mut();
    while let Some(pkt) = q.pop_front() {
        sock.send_to(&pkt, dest).expect("send_to ok");
    }
}

// ── naive: sleep-based update() ──────────────────────────────────────────────

fn bench_kcp_naive(c: &mut Criterion) {
    let (cli_core, srv_core) = pick_cores();
    let srv_sock = UdpSocket::bind("127.0.0.1:0").expect("srv bind");
    srv_sock.set_nonblocking(true).expect("srv nonblock");
    let srv_addr = srv_sock.local_addr().expect("srv addr");

    let cli_sock = UdpSocket::bind("127.0.0.1:0").expect("cli bind");
    cli_sock.set_nonblocking(true).expect("cli nonblock");
    let cli_addr = cli_sock.local_addr().expect("cli addr");

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);

    let srv_sock2 = srv_sock.try_clone().expect("srv clone");
    let srv_ready = Arc::new(AtomicBool::new(false));
    let srv_ready2 = Arc::clone(&srv_ready);

    let srv_handle = thread::spawn(move || {
        core_affinity::set_for_current(srv_core);
        let srv_out: OutQueue = Rc::new(RefCell::new(VecDeque::new()));
        let mut kcp = make_kcp(1, Rc::clone(&srv_out));
        let mut buf = [0u8; 2048];
        let mut msg = [0u8; 2048];
        srv_ready2.store(true, Ordering::Release);
        while !stop2.load(Ordering::Relaxed) {
            loop {
                match srv_sock2.recv_from(&mut buf) {
                    Ok((n, _)) => {
                        kcp.input(&buf[..n]).expect("server kcp.input");
                    }
                    Err(_) => break,
                }
            }
            while let Ok(n) = kcp.recv(&mut msg) {
                kcp.send(&msg[..n]).expect("server kcp.send");
            }
            kcp.update(now_ms()).expect("server kcp.update");
            drain_output(&srv_out, &srv_sock2, cli_addr);
            thread::sleep(Duration::from_millis(1));
        }
    });

    // Client side runs the Criterion timer on this thread.
    core_affinity::set_for_current(cli_core);

    // Wait for server thread to publish ready before client touches the wire.
    while !srv_ready.load(Ordering::Acquire) {
        std::hint::spin_loop();
    }

    let cli_out: OutQueue = Rc::new(RefCell::new(VecDeque::new()));
    let mut kcp = make_kcp(1, Rc::clone(&cli_out));
    let mut udp_buf = [0u8; 2048];
    let mut msg_buf = [0u8; 2048];

    // Warmup: keep retrying until first echo arrives, no silent break.
    let warmup_payload = [0x42u8; PAYLOAD_LEN];
    kcp.send(&warmup_payload).expect("warmup send");
    kcp.update(now_ms()).expect("warmup update");
    drain_output(&cli_out, &cli_sock, srv_addr);
    let warmup_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if Instant::now() > warmup_deadline {
            panic!("KCP naive warmup timed out — server thread not echoing");
        }
        while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) {
            kcp.input(&udp_buf[..n]).expect("warmup input");
        }
        if kcp.recv(&mut msg_buf).is_ok() {
            break;
        }
        kcp.update(now_ms()).expect("warmup update loop");
        drain_output(&cli_out, &cli_sock, srv_addr);
        thread::sleep(Duration::from_millis(1));
    }

    let payload = [0xAAu8; PAYLOAD_LEN];
    c.bench_function("kcp_rtt_naive_1ms_interval_128b", |b| {
        b.iter(|| {
            kcp.send(black_box(&payload)).expect("iter send");
            kcp.update(now_ms()).expect("iter update");
            drain_output(&cli_out, &cli_sock, srv_addr);
            loop {
                while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) {
                    kcp.input(&udp_buf[..n]).expect("iter input");
                }
                if kcp.recv(&mut msg_buf).is_ok() {
                    black_box(&msg_buf);
                    break;
                }
                kcp.update(now_ms()).expect("iter update loop");
                drain_output(&cli_out, &cli_sock, srv_addr);
                thread::sleep(Duration::from_millis(1));
            }
        });
    });

    stop.store(true, Ordering::Release);
    srv_handle.join().expect("server join");
}

// ── spin: flush() immediately, no sleep ──────────────────────────────────────

fn bench_kcp_spin(c: &mut Criterion) {
    let (cli_core, srv_core) = pick_cores();
    let srv_sock = UdpSocket::bind("127.0.0.1:0").expect("srv bind");
    srv_sock.set_nonblocking(true).expect("srv nonblock");
    let srv_addr = srv_sock.local_addr().expect("srv addr");

    let cli_sock = UdpSocket::bind("127.0.0.1:0").expect("cli bind");
    cli_sock.set_nonblocking(true).expect("cli nonblock");
    let cli_addr = cli_sock.local_addr().expect("cli addr");

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);

    let srv_sock2 = srv_sock.try_clone().expect("srv clone");
    let srv_ready = Arc::new(AtomicBool::new(false));
    let srv_ready2 = Arc::clone(&srv_ready);

    let srv_handle = thread::spawn(move || {
        core_affinity::set_for_current(srv_core);
        let srv_out: OutQueue = Rc::new(RefCell::new(VecDeque::new()));
        let mut kcp = make_kcp(2, Rc::clone(&srv_out));
        let mut buf = [0u8; 2048];
        let mut msg = [0u8; 2048];
        // The Rust `kcp` crate requires at least one `update()` call
        // before `flush()` will run (otherwise it returns NeedUpdate).
        // We pay this once at startup; the flush() calls in the hot
        // loop then bypass the scheduler.
        kcp.update(now_ms()).expect("server kcp.update bootstrap");
        srv_ready2.store(true, Ordering::Release);
        // Spin: no sleep. Process every inbound frame immediately;
        // emit the echo + flush so DATA goes out without waiting
        // for the update() scheduler.
        while !stop2.load(Ordering::Relaxed) {
            let mut got = false;
            while let Ok((n, _)) = srv_sock2.recv_from(&mut buf) {
                kcp.input(&buf[..n]).expect("server kcp.input");
                got = true;
            }
            if got {
                while let Ok(n) = kcp.recv(&mut msg) {
                    kcp.send(&msg[..n]).expect("server kcp.send");
                }
                // flush() emits any pending DATA + ACK frames immediately.
                kcp.flush().expect("server kcp.flush");
                drain_output(&srv_out, &srv_sock2, cli_addr);
            } else {
                std::hint::spin_loop();
            }
        }
    });

    // Receiver-ready barrier: confirmed before the timed loop starts.
    while !srv_ready.load(Ordering::Acquire) {
        std::hint::spin_loop();
    }

    // Client side runs the Criterion timer on this thread.
    core_affinity::set_for_current(cli_core);

    let cli_out: OutQueue = Rc::new(RefCell::new(VecDeque::new()));
    let mut kcp = make_kcp(2, Rc::clone(&cli_out));
    let mut udp_buf = [0u8; 2048];
    let mut msg_buf = [0u8; 2048];

    // Bootstrap `updated` flag so subsequent flush() doesn't return
    // NeedUpdate. See server-side comment.
    kcp.update(now_ms()).expect("client kcp.update bootstrap");

    // Warmup: full RTT must complete before timing begins.
    let warmup_payload = [0x42u8; PAYLOAD_LEN];
    kcp.send(&warmup_payload).expect("warmup send");
    kcp.flush().expect("warmup flush");
    drain_output(&cli_out, &cli_sock, srv_addr);
    let warmup_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if Instant::now() > warmup_deadline {
            panic!("KCP spin warmup timed out — server thread not echoing");
        }
        while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) {
            kcp.input(&udp_buf[..n]).expect("warmup input");
        }
        if kcp.recv(&mut msg_buf).is_ok() {
            // Flush the ACK back so the server can release the segment
            // before our next send (otherwise the standalone ACK is
            // deferred until the next outbound DATA frame).
            kcp.flush().expect("warmup ack flush");
            drain_output(&cli_out, &cli_sock, srv_addr);
            break;
        }
        std::hint::spin_loop();
    }

    let payload = [0xAAu8; PAYLOAD_LEN];
    c.bench_function("kcp_rtt_spin_flush_128b", |b| {
        b.iter(|| {
            kcp.send(black_box(&payload)).expect("iter send");
            // flush() sends the DATA frame immediately, bypassing
            // the update() scheduler.
            kcp.flush().expect("iter flush send");
            drain_output(&cli_out, &cli_sock, srv_addr);
            // Spin until the echo DATA frame arrives.
            loop {
                while let Ok((n, _)) = cli_sock.recv_from(&mut udp_buf) {
                    kcp.input(&udp_buf[..n]).expect("iter input");
                }
                if kcp.recv(&mut msg_buf).is_ok() {
                    // Send the standalone ACK back to the server so
                    // its retransmit timer doesn't fire. We flush
                    // AFTER the recv so the timed RTT does include
                    // the ACK emission cost of the previous iter's
                    // echo — making the ACK path measurable rather
                    // than deferred into the next iter's bookkeeping.
                    kcp.flush().expect("iter ack flush");
                    drain_output(&cli_out, &cli_sock, srv_addr);
                    black_box(&msg_buf);
                    break;
                }
                std::hint::spin_loop();
            }
        });
    });

    stop.store(true, Ordering::Release);
    srv_handle.join().expect("server join");
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_kcp_naive, bench_kcp_spin
}
criterion_main!(benches);
