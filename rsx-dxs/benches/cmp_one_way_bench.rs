//! CMP one-way latency: `CmpSender::send` → `CmpReceiver::try_recv`.
//!
//! What this measures
//! -----------------
//! `CmpSender::send` returns to `CmpReceiver::try_recv` returns
//! the same record. Single direction, one CMP hop, in-order,
//! no NAK. Both sides on 127.0.0.1, both threads spinning
//! cache-hot.
//!
//! This is the protocol isolation of the production legs
//! `gateway_in → risk_in` and `risk_out → gateway_cmp_recv`.
//! Adds (over `udp_rtt_loopback_64b`): WalHeader build + CRC32
//! over the 128-byte FillRecord, send-ring slot cache,
//! receiver-side WalHeader parse + CRC32 verify + in-order
//! seq accounting.
//!
//! Happy path only — no NAK injection, no socket failure, no
//! sender restart. Producer/consumer flow control is kept
//! open by periodic `sender.tick()` calls (every 1024 iters)
//! which trigger peer status round-trips. Without that, the
//! sender's window closes around iter 65536 and `send` returns
//! `Ok(false)` forever. (The bench is exercising sustained
//! throughput; production has the same heartbeat cadence
//! built into the gateway's main loop.)
//!
//! Caveats
//! -------
//! - Both sides on the same host: real net adds NIC IRQ +
//!   driver tx/rx + (sometimes) interrupt-coalescing delay.
//! - The receiver allocates a `Vec<u8>` per in-order delivery
//!   (CmpReceiver::try_recv is NOT zero-heap on recv; the
//!   zero-heap claim applies to CmpSender::send only).

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
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn bench_cmp_one_way(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let send_bind = ephemeral_addr();
    let recv_bind = ephemeral_addr();

    let mut sender = CmpSender::with_config(
        recv_bind,
        1,
        tmp.path(),
        &rsx_dxs::config::CmpConfig {
            sender_bind_addr: Some(send_bind.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver =
        CmpReceiver::new(recv_bind, sender_addr, 1).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let recv_count = Arc::new(AtomicU64::new(0));
    let stop_clone = Arc::clone(&stop);
    let recv_clone = Arc::clone(&recv_count);

    // Receiver worker: spin try_recv + drive `tick()` so the
    // peer status replies the sender needs to keep its window
    // open get sent.
    let handle = thread::spawn(move || {
        let mut i: u64 = 0;
        while !stop_clone.load(Ordering::Relaxed) {
            if let Some(frame) = receiver.try_recv() {
                black_box(frame);
                recv_clone.fetch_add(1, Ordering::Release);
            } else {
                std::hint::spin_loop();
            }
            // Tick periodically: sends status messages that
            // advance the sender's peer_consumption_seq, keeping
            // the flow-control window open.
            i = i.wrapping_add(1);
            if i & 0x3FF == 0 {
                receiver.tick();
            }
        }
    });

    c.bench_function("cmp_one_way_fill", |b| {
        let mut iter: u64 = 0;
        b.iter(|| {
            let mut rec = fill_record();
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
            loop {
                match sender.send(black_box(&mut rec)) {
                    Ok(true) => break,
                    Ok(false) => {
                        // Window closed — drive a tick to
                        // pick up peer status, then retry.
                        let _ = sender.tick();
                        sender.recv_control();
                        std::hint::spin_loop();
                    }
                    Err(e) => panic!("send: {e}"),
                }
            }
            while recv_count.load(Ordering::Acquire) == before {
                std::hint::spin_loop();
            }
        });
    });

    stop.store(true, Ordering::Release);
    let _ = handle.join();
}

criterion_group!(benches, bench_cmp_one_way);
criterion_main!(benches);
