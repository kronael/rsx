//! CMP echo round-trip: A → B → A.
//!
//! What this measures
//! -----------------
//! Two CMP endpoints. A sends a `FillRecord` to B; B receives,
//! echoes the (modified) record back; A receives. Timed: the
//! full round-trip from A's `send()` return to A's `try_recv()`
//! returning B's echo.
//!
//! This is closer in shape to the production
//! `gateway_in → risk_in → gateway_cmp_recv` triangle than a
//! one-way bench: it includes two CMP send paths, two CMP
//! recv paths, and the natural bidirectional traffic that the
//! NAK/STATUS subsystem expects. We still do NOT call `tick()`
//! on either side; heartbeat/status cost is not on the per-
//! packet critical path.
//!
//! Each side runs a fresh CmpSender + CmpReceiver pair:
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
//!   so the two CmpSenders don't share `wal_dir` and accidentally
//!   alias their NAK-fallback paths.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
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
    let mut a_sender = CmpSender::with_config(
        b_recv_bind,
        1,
        tmp_a.path(),
        &rsx_dxs::config::CmpConfig {
            sender_bind_addr: Some(a_send_bind.to_string()),
            ..rsx_dxs::config::CmpConfig::default()
        },
    )
    .unwrap();

    // A.receiver expects traffic from B.sender
    let mut a_receiver =
        CmpReceiver::new(a_recv_bind, b_send_bind, 1).unwrap();

    // B.sender -> A.receiver
    let mut b_sender = CmpSender::with_config(
        a_recv_bind,
        2,
        tmp_b.path(),
        &rsx_dxs::config::CmpConfig {
            sender_bind_addr: Some(b_send_bind.to_string()),
            ..rsx_dxs::config::CmpConfig::default()
        },
    )
    .unwrap();

    // B.receiver expects traffic from A.sender
    let mut b_receiver =
        CmpReceiver::new(b_recv_bind, a_send_bind, 2).unwrap();

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
    let handle = thread::spawn(move || {
        core_affinity::set_for_current(b_core);
        let mut i: u64 = 0;
        while !stop_b.load(Ordering::Relaxed) {
            if let Some(_) = b_receiver.try_recv() {
                let mut echo = fill_record();
                loop {
                    match b_sender.send(&mut echo) {
                        Ok(true) => break,
                        Ok(false) => {
                            let _ = b_sender.tick();
                            b_sender.recv_control();
                            std::hint::spin_loop();
                        }
                        Err(e) => panic!("b send: {e}"),
                    }
                }
                echoes_b.fetch_add(1, Ordering::Release);
            } else {
                std::hint::spin_loop();
            }
            i = i.wrapping_add(1);
            if i & 0x3FF == 0 {
                b_receiver.tick();
                let _ = b_sender.tick();
                b_sender.recv_control();
            }
        }
    });

    // Sender side runs the Criterion timer closure on this thread.
    core_affinity::set_for_current(a_core);

    c.bench_function("cmp_rtt_fill_echo", |b| {
        let mut iter: u64 = 0;
        b.iter(|| {
            iter = iter.wrapping_add(1);
            if iter & 0x3FF == 0 {
                a_receiver.tick();
                let _ = a_sender.tick();
                a_sender.recv_control();
            }
            let mut req = fill_record();
            loop {
                match a_sender.send(black_box(&mut req)) {
                    Ok(true) => break,
                    Ok(false) => {
                        let _ = a_sender.tick();
                        a_sender.recv_control();
                        std::hint::spin_loop();
                    }
                    Err(e) => panic!("a send: {e}"),
                }
            }
            loop {
                if let Some(reply) = a_receiver.try_recv() {
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

criterion_group!(benches, bench_cmp_rtt);
criterion_main!(benches);
