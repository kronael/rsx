//! casting one-way latency: `CastSender::send` → `CastReceiver::try_recv`.
//!
//! What this measures
//! -----------------
//! `CastSender::send` returns to `CastReceiver::try_recv` returns
//! the same record. Single direction, one cast hop, in-order,
//! no NAK. Both sides on 127.0.0.1, both threads spinning
//! cache-hot.
//!
//! Sender pinned to core 2, receiver to core 3. Without pinning
//! the threads migrate and the one-way latency picks up µs-scale
//! cache-eviction noise.
//!
//! This is the protocol isolation of the production legs
//! `gateway_in → risk_in` and `risk_out → gateway_cast_recv`.
//! Adds (over `udp_rtt_loopback_64b`): WalHeader build + CRC32
//! over the 128-byte FillRecord, send-ring slot cache,
//! receiver-side WalHeader parse + CRC32 verify + in-order
//! seq accounting.
//!
//! Happy path only — no NAK injection, no socket failure, no
//! sender restart. `sender.tick()` is invoked every 1024 iters
//! to emit idle-stream heartbeats (the sender's send() also
//! resets the heartbeat timer; the explicit tick is defensive).
//!
//! Caveats
//! -------
//! - Both sides on the same host: real net adds NIC IRQ +
//!   driver tx/rx + (sometimes) interrupt-coalescing delay.
//! - The receiver allocates a `Vec<u8>` per in-order delivery
//!   (CastReceiver::try_recv is NOT zero-heap on recv; the
//!   zero-heap claim applies to CastSender::send only).

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
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let s = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let r = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (s, r)
}

fn bench_cmp_one_way(c: &mut Criterion) {
    let (sender_core, recv_core) = pick_cores();
    let tmp = TempDir::new().unwrap();
    let send_bind = ephemeral_addr();
    let recv_bind = ephemeral_addr();

    let mut sender = CastSender::with_config(
        recv_bind,
        1,
        tmp.path(),
        &rsx_cast::config::CastConfig {
            sender_bind_addr: Some(send_bind.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver = CastReceiver::new(recv_bind, sender_addr).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let recv_count = Arc::new(AtomicU64::new(0));
    let stop_clone = Arc::clone(&stop);
    let recv_clone = Arc::clone(&recv_count);

    // Receiver worker: spin try_recv.
    let handle = thread::spawn(move || {
        core_affinity::set_for_current(recv_core);
        while !stop_clone.load(Ordering::Relaxed) {
            if let CastRecv::Data(_, payload) = receiver.try_recv() {
                black_box(payload);
                recv_clone.fetch_add(1, Ordering::Release);
            } else {
                std::hint::spin_loop();
            }
        }
    });

    // Sender side runs the Criterion timer closure on this thread.
    core_affinity::set_for_current(sender_core);

    // Pre-build the record outside the timed loop (seq is
    // overwritten by sender.send).
    let mut rec = fill_record();
    c.bench_function("cmp_one_way_fill", |b| {
        let mut iter: u64 = 0;
        b.iter(|| {
            let before = recv_count.load(Ordering::Acquire);
            // Send. Flow control closes around 64K iters
            // without status round-trips — we drive
            // `sender.tick()` once per 1024 sends so the
            // sender can recv status from the receiver and
            // advance peer_consumption_seq.
            iter = iter.wrapping_add(1);
            if iter & 0x3FF == 0 {
                let _ = sender.tick();
                sender.recv_control();
            }
            let framed = Framed::pack(black_box(&mut rec), iter);
            if let Err(e) = sender.send_framed(&framed) {
                panic!("send: {e}");
            }
            while recv_count.load(Ordering::Acquire) == before {
                std::hint::spin_loop();
            }
        });
    });

    stop.store(true, Ordering::Release);
    let _ = handle.join();
}

criterion_group! {
    name = benches;
    // sample_size(50) matches the rest of the compare/cast RTT bench
    // family for cross-bench alignment.
    config = Criterion::default().sample_size(50);
    targets = bench_cmp_one_way
}
criterion_main!(benches);
