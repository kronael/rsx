//! UDP loopback round-trip baseline.
//!
//! What this measures
//! -----------------
//! Two `std::net::UdpSocket`s on 127.0.0.1, in non-blocking
//! mode, both threads cache-hot and spinning, **pinned to
//! cores 2 (sender) and 3 (echoer)** so neither thread migrates
//! mid-iteration. No CMP, no framing, no CRC, no WAL — just
//! `send_to` → spin `recv_from` → `send_to` (echo) → spin
//! `recv_from`. Payload is **128 bytes** to match `FillRecord` (the
//! CMP record exercised by `cmp_rtt_bench`). Earlier versions used
//! 64 bytes, which was apples-to-oranges against CMP RTT.
//!
//! The harness matches the in-process `bench-e2e-pipeline`
//! pattern: both threads pre-warmed, no per-iteration
//! `setsockopt`, no blocking syscall wake-up, no stop-channel
//! poll. The previous version called `set_read_timeout`
//! inside the echo loop on every iteration — that added a
//! setsockopt syscall (~µs) and a blocking-recv wake-up to
//! every measured round-trip, overstating the true UDP RTT
//! by ~3×.
//!
//! This is the absolute lower bound for any CMP-based RTT
//! between two endpoints on the same host. Anything CMP adds
//! (framing, CRC32, send-ring caching, NAK/heartbeat ticks,
//! reorder buffer, peer_consumption_seq flow control) shows
//! up as overhead above this number.
//!
//! Assumptions / caveats
//! --------------------
//! - Loopback, not real net (no PHY/MAC, no driver, no NIC IRQ).
//! - Non-blocking sockets + busy-spin. The production gateway
//!   does NOT busy-spin; it does `try_recv` + a periodic yield.
//!   So real production overhead per packet is higher than
//!   this bench's RTT — that's the bench's point: this is
//!   the floor.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

/// Pick two distinct cores for sender + echoer. Falls back to
/// cores 0/1 on hosts with fewer than 4 reported cores.
fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let sender = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let echoer = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (sender, echoer)
}

fn bench_udp_rtt_loopback(c: &mut Criterion) {
    let (sender_core, echoer_core) = pick_cores();

    // Both sockets bound up-front; bind never on the hot path.
    // Pinning happens AFTER bind so the kernel allocates per-CPU
    // socket buffers on a settled node.
    let echoer = UdpSocket::bind("127.0.0.1:0").unwrap();
    let echoer_addr = echoer.local_addr().unwrap();
    let pinger = UdpSocket::bind("127.0.0.1:0").unwrap();

    echoer.set_nonblocking(true).unwrap();
    pinger.set_nonblocking(true).unwrap();

    // Echoer thread: pinned, spin on non-blocking recv_from,
    // echo straight back. Sentinel byte 0xFF exits the loop.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = thread::spawn(move || {
        core_affinity::set_for_current(echoer_core);
        let mut buf = [0u8; 128];
        while !stop_clone.load(Ordering::Relaxed) {
            match echoer.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if n >= 1 && buf[0] == 0xFF {
                        return;
                    }
                    let _ = echoer.send_to(&buf[..n], src);
                }
                Err(ref e)
                    if e.kind()
                        == std::io::ErrorKind::WouldBlock =>
                {
                    std::hint::spin_loop();
                }
                Err(_) => return,
            }
        }
    });

    // Sender (this thread runs the Criterion measurement closure).
    core_affinity::set_for_current(sender_core);

    let payload = [0xAAu8; 128];
    let mut recv_buf = [0u8; 256];

    c.bench_function("udp_rtt_loopback_128b", |b| {
        b.iter(|| {
            pinger
                .send_to(black_box(&payload), echoer_addr)
                .unwrap();
            // Spin until the echo arrives. Non-blocking
            // recv_from + spin keeps both threads cache-hot.
            // Panic on non-WouldBlock so a dead echoer can't
            // silently record near-zero iterations.
            loop {
                match pinger.recv_from(&mut recv_buf) {
                    Ok((n, _)) => {
                        black_box(n);
                        break;
                    }
                    Err(ref e)
                        if e.kind()
                            == std::io::ErrorKind::WouldBlock =>
                    {
                        std::hint::spin_loop();
                    }
                    Err(e) => panic!("pinger recv: {e}"),
                }
            }
        });
    });

    // Signal exit + flush.
    stop.store(true, Ordering::Release);
    let _ = pinger.send_to(&[0xFFu8; 1], echoer_addr);
    let _ = handle.join();
}

criterion_group! {
    name = benches;
    // sample_size(50) matches compare_tcp / compare_aeron / compare_kcp.
    config = Criterion::default().sample_size(50);
    targets = bench_udp_rtt_loopback
}
criterion_main!(benches);
