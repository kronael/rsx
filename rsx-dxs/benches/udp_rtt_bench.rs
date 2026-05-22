//! UDP loopback round-trip baseline.
//!
//! What this measures
//! -----------------
//! Two `std::net::UdpSocket`s bound to 127.0.0.1, no CMP, no
//! framing, no CRC, no WAL — just send → recv → echo → recv.
//! Payload is 64 bytes (one cache line), matching the
//! `#[repr(C, align(64))]` records that flow through the
//! real CMP path.
//!
//! This is the absolute lower bound for any CMP-based RTT
//! between two processes on the same host. Anything CMP adds
//! (framing, CRC32, send-ring caching, NAK/heartbeat ticks,
//! reorder buffer) shows up as overhead above this number.
//!
//! Assumptions / caveats
//! --------------------
//! - Loopback is faster than a real network: no PHY/MAC, no
//!   driver tx/rx queue, no NIC IRQ. Real LAN RTT will be
//!   higher even with the same protocol overhead.
//! - Two threads, blocking `recv_from` on each. We use a
//!   `Barrier` to keep the echoer ready before each send.
//! - Both sockets are blocking. Non-blocking + busy-spin
//!   would shave a few hundred nanoseconds but is not how
//!   the production gateway is wired.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use std::net::UdpSocket;
use std::sync::mpsc;
use std::thread;

fn bench_udp_rtt_loopback(c: &mut Criterion) {
    // Bind both sockets up-front so the bench loop never
    // touches the bind() syscall.
    let echoer = UdpSocket::bind("127.0.0.1:0").unwrap();
    let echoer_addr = echoer.local_addr().unwrap();
    let pinger = UdpSocket::bind("127.0.0.1:0").unwrap();
    let pinger_addr = pinger.local_addr().unwrap();

    // Echoer thread: blocking recv_from, send_to back.
    // Stop on a 1-byte "X" payload.
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 128];
        loop {
            if stop_rx.try_recv().is_ok() {
                return;
            }
            // Short timeout so we can poll the stop channel
            // without leaving the recv blocked forever after
            // the bench finishes.
            echoer.set_read_timeout(Some(
                std::time::Duration::from_millis(50),
            ))
            .unwrap();
            match echoer.recv_from(&mut buf) {
                Ok((n, src)) => {
                    echoer.send_to(&buf[..n], src).unwrap();
                }
                Err(_) => continue,
            }
        }
    });

    let payload = [0xAAu8; 64];
    let mut recv_buf = [0u8; 128];

    c.bench_function("udp_rtt_loopback_64b", |b| {
        b.iter(|| {
            pinger
                .send_to(black_box(&payload), echoer_addr)
                .unwrap();
            let (n, _) = pinger.recv_from(&mut recv_buf).unwrap();
            black_box(n);
        });
    });

    // Tell the echoer to exit, then ping it once so the
    // pending recv_from returns immediately.
    let _ = stop_tx.send(());
    let _ = pinger.send_to(&[0u8; 1], echoer_addr);
    let _ = pinger_addr;
    let _ = handle.join();
}

criterion_group!(benches, bench_udp_rtt_loopback);
criterion_main!(benches);
