//! CMP one-way latency: `CmpSender::send` → `CmpReceiver::try_recv`.
//!
//! What this measures
//! -----------------
//! End-to-end time from the sender's `send()` returning Ok
//! to the receiver's `try_recv()` returning the in-order
//! record, on the same host over UDP loopback. The receiver
//! spins on `try_recv()` from a dedicated thread; the bench
//! thread is the sender.
//!
//! This is the protocol-level isolation of the legs
//! `gateway_in → risk_in` and `risk_out → gateway_cmp_recv`
//! (single CMP hop, in-order, no NAK).
//!
//! Includes:
//! - Header build + CRC32 over a 128-byte FillRecord payload
//! - UDP send_to (loopback)
//! - UDP recv_from (loopback)
//! - WalHeader parse + CRC32 verify on receiver
//! - Send-ring slot write (preallocated)
//!
//! Excludes:
//! - Heartbeat / status / NAK traffic (`tick()` never called
//!   in the hot loop)
//! - WAL append (CmpSender does not write to WAL; WAL is the
//!   recorder's job)
//!
//! Assumptions / caveats
//! --------------------
//! - Loopback, not real net (see udp_rtt_bench).
//! - Receiver allocates a `Vec<u8>` per in-order delivery
//!   (documented non-zero-heap on the recv path). On a real
//!   GW→ME→GW hot path, the receiver thread would copy into
//!   a typed slot or push into a SPSC ring; here we just
//!   black_box the returned Vec, which is the same shape as
//!   the current risk tile's CMP consumer.
//! - Sender thread does NOT call `tick()`; in production
//!   `tick()` is on a 10ms cadence and is not on the per-
//!   packet critical path.

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

fn bench_cmp_one_way(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();

    // Pick two free ports up front.
    let sender_bind = ephemeral_addr();
    let receiver_bind = ephemeral_addr();

    // Sender uses sender_bind, dest = receiver_bind.
    let mut sender = CmpSender::with_config(
        receiver_bind,
        1,
        tmp.path(),
        &rsx_dxs::config::CmpConfig {
            sender_bind_addr: Some(sender_bind.to_string()),
            ..rsx_dxs::config::CmpConfig::default()
        },
    )
    .unwrap();

    // Receiver bound to receiver_bind, sender_addr = where
    // it sends NAKs/STATUS (back to the sender's bind).
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver =
        CmpReceiver::new(receiver_bind, sender_addr, 1)
            .unwrap();

    // Recv side: tight spin in a worker thread. The worker
    // counts every in-order record it sees; the sender
    // bench iter publishes a seq and waits for the counter
    // to advance past it.
    let stop = Arc::new(AtomicBool::new(false));
    let recv_count = Arc::new(AtomicU64::new(0));

    let stop_clone = Arc::clone(&stop);
    let recv_clone = Arc::clone(&recv_count);

    let handle = thread::spawn(move || {
        while !stop_clone.load(Ordering::Relaxed) {
            if let Some((hdr, data)) = receiver.try_recv() {
                black_box((hdr, data));
                recv_clone.fetch_add(1, Ordering::Release);
            } else {
                std::hint::spin_loop();
            }
        }
    });

    c.bench_function("cmp_one_way_fill", |b| {
        b.iter(|| {
            let mut rec = fill_record();
            let before = recv_count.load(Ordering::Acquire);
            // Loop until send succeeds (flow control / send
            // ring should never stall in this setup but be
            // safe).
            loop {
                match sender.send(black_box(&mut rec)) {
                    Ok(true) => break,
                    Ok(false) => {
                        std::hint::spin_loop();
                    }
                    Err(e) => panic!("send failed: {e}"),
                }
            }
            // Wait for receiver to pick this one up.
            while recv_count.load(Ordering::Acquire) == before {
                std::hint::spin_loop();
            }
        });
    });

    stop.store(true, Ordering::Release);
    // Send one more frame to unstick the receiver in case
    // it's spinning on an empty socket — actually try_recv
    // is non-blocking so it just polls; the stop flag is
    // enough.
    let _ = handle.join();
}

criterion_group!(benches, bench_cmp_one_way);
criterion_main!(benches);
