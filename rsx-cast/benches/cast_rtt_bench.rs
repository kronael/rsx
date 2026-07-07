//! casting echo round-trip: A → B → A.
//!
//! What this measures
//! -----------------
//! Two cast endpoints. A sends a `FillRecord` to B; B receives,
//! echoes the (modified) record back; A receives. Timed: the
//! full round-trip from A's `send()` return to A's `try_recv()`
//! returning B's echo.
//!
//! This is closer in shape to the production
//! `gateway_in → risk_in → gateway_cast_recv` triangle than a
//! one-way bench: it includes two cast send paths, two cast
//! recv paths, and the natural bidirectional traffic that the
//! NAK subsystem expects. Senders call `tick()` every 1024
//! iters to emit idle-stream heartbeats; the heartbeat path
//! is off the per-packet critical path.
//!
//! Each side runs a fresh CastSender + CastReceiver pair:
//!   A.sender → B.receiver
//!   B.sender → A.receiver
//!
//! Threads pinned: side A (the timed Criterion thread) on core 2,
//! side B (the echoer) on core 3. Without pinning the threads
//! migrate mid-bench and the RTT distribution gets µs-wide tails
//! from L1/L2 eviction.
//!
//! Assumptions / caveats
//! --------------------
//! - Loopback, see `udp_rtt_bench`.
//! - The B-side echo thread spins on `try_recv()` and sends
//!   immediately. No additional work between recv and send
//!   (no risk validation, no WAL append).
//! - We initialize two distinct WAL dirs (one per sender)
//!   so the two CastSenders don't share `wal_dir` and accidentally
//!   alias their NAK-fallback paths.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::Framed;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

fn fill_record() -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: Price(50_000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    }
}

fn ephemeral_addr() -> SocketAddr {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.local_addr().unwrap()
}

fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let a = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let b = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (a, b)
}

fn bench_cmp_rtt(c: &mut Criterion) {
    let (a_core, b_core) = pick_cores();
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();

    // Pick four free ports up front: A.sender, A.receiver,
    // B.sender, B.receiver.
    let a_send_bind = ephemeral_addr();
    let a_recv_bind = ephemeral_addr();
    let b_send_bind = ephemeral_addr();
    let b_recv_bind = ephemeral_addr();

    // A.sender -> B.receiver
    let mut a_sender = CastSender::with_config(
        b_recv_bind,
        1,
        tmp_a.path(),
        &rsx_cast::config::CastConfig {
            sender_bind_addr: Some(a_send_bind.to_string()),
            ..rsx_cast::config::CastConfig::default()
        },
    )
    .unwrap();

    // A.receiver expects traffic from B.sender
    let mut a_receiver = CastReceiver::new(a_recv_bind, b_send_bind).unwrap();

    // B.sender -> A.receiver
    let mut b_sender = CastSender::with_config(
        a_recv_bind,
        2,
        tmp_b.path(),
        &rsx_cast::config::CastConfig {
            sender_bind_addr: Some(b_send_bind.to_string()),
            ..rsx_cast::config::CastConfig::default()
        },
    )
    .unwrap();

    // B.receiver expects traffic from A.sender
    let mut b_receiver = CastReceiver::new(b_recv_bind, a_send_bind).unwrap();

    // B-side echo thread: try_recv → b_sender.send back.
    let stop = Arc::new(AtomicBool::new(false));
    let echoes = Arc::new(AtomicU64::new(0));

    let stop_b = Arc::clone(&stop);
    let echoes_b = Arc::clone(&echoes);

    // B-side echo: spin try_recv → send back. Periodic tick
    // on both receiver and sender keeps the flow-control
    // windows open across sustained Criterion measurement
    // (~100k+ iterations) — without it the senders' windows
    // close around iter 65536 and `send` returns Ok(false)
    // forever.
    //
    // The echo record is pre-built outside the inner loop and
    // re-used (seq is set by the sender). This shaves the
    // per-iter struct construction (~10s ns) off the echo path.
    let handle = thread::spawn(move || {
        core_affinity::set_for_current(b_core);
        let mut echo = fill_record();
        let mut b_seq: u64 = 0;
        let mut i: u64 = 0;
        while !stop_b.load(Ordering::Relaxed) {
            if let CastRecv::Data(_, _) = b_receiver.try_recv() {
                b_seq += 1;
                let framed = Framed::pack(&mut echo, b_seq);
                if let Err(e) = b_sender.send_framed(&framed) {
                    panic!("b send: {e}");
                }
                echoes_b.fetch_add(1, Ordering::Release);
            } else {
                std::hint::spin_loop();
            }
            i = i.wrapping_add(1);
            if i & 0x3FF == 0 {
                let _ = b_sender.tick();
                b_sender.recv_control();
            }
        }
    });

    // Sender side runs the Criterion timer closure on this thread.
    core_affinity::set_for_current(a_core);

    // Pre-build the request record outside b.iter (seq is
    // overwritten on send).
    let mut req = fill_record();
    c.bench_function("cmp_rtt_fill_echo", |b| {
        let mut iter: u64 = 0;
        b.iter(|| {
            iter = iter.wrapping_add(1);
            if iter & 0x3FF == 0 {
                let _ = a_sender.tick();
                a_sender.recv_control();
            }
            let framed = Framed::pack(black_box(&mut req), iter);
            if let Err(e) = a_sender.send_framed(&framed) {
                panic!("a send: {e}");
            }
            loop {
                if let CastRecv::Data(_, reply) = a_receiver.try_recv() {
                    black_box(reply);
                    break;
                }
                std::hint::spin_loop();
            }
        });
    });

    stop.store(true, Ordering::Release);
    let _ = handle.join();
}

criterion_group! {
    name = benches;
    // sample_size(50) matches compare_udp / compare_tcp / compare_aeron /
    // compare_kcp so cross-bench tables align on sampling methodology.
    config = Criterion::default().sample_size(50);
    targets = bench_cmp_rtt
}
criterion_main!(benches);
