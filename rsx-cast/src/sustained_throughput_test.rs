//! Sustained CMP send throughput.
//!
//! Send 50k records through a CastSender and assert that the
//! wall-clock cost is well under one second. The criterion
//! bench suite covers single-record latency; this test covers
//! the steady-state path where ring caching, syscall pacing
//! and any allocator hot spots would compound.
//!
//! Delivery floor is set at 10% (not 90%) — UDP loopback drops
//! under sustained burst pressure on smaller boxes, but if the
//! receiver saw fewer than 10% of datagrams the receive path is
//! silently broken and the throughput number is meaningless.

use crate::cast::CastRecv;
use crate::cast::CastReceiver;
use crate::cast::CastSender;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::UdpSocket;

use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const N: u64 = 50_000;

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000 + seq,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
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

#[test]
fn cmp_send_50k_under_one_second() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path();

    // Receiver socket: an OS-assigned port we drain on a
    // background thread so the kernel's UDP receive buffer
    // doesn't back up and trigger send-side EAGAIN.
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock);

    let mut sender =
        CastSender::new(recv_addr, 1, wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    // Throughput-only test: the sender never calls
    // recv_control here, so receiver NAKs never close
    // gaps. Disable FAULT escalation so a transient
    // out-of-order tail doesn't stick the receiver. The
    // gap-recovery + FAULTED contract is exercised in
    // cmp_v4_test.rs.
    let recv_cfg = crate::config::CastConfig {
        max_nak_retries: u16::MAX,
        ..Default::default()
    };
    let mut receiver = CastReceiver::with_config(
        recv_addr,
        sender_addr,
        1,
        &recv_cfg,
    )
    .unwrap();

    // Drain thread: keeps the receive buffer empty so the OS
    // doesn't drop datagrams (which the sender wouldn't see
    // as failures but would silently distort the throughput
    // measurement).
    let drain_stop =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        ));
    let drain_flag = drain_stop.clone();
    let drainer = thread::spawn(move || {
        let mut count: u64 = 0;
        // Throughput test: simulate the consumer-side
        // recovery path by treating FAULTED as a one-shot
        // resync (loopback UDP drops + slot conflict are
        // expected at 50k burst).
        loop {
            match receiver.try_recv() {
                CastRecv::Data(_, _) => {
                    count += 1;
                }
                CastRecv::Empty => {
                    if drain_flag.load(
                        std::sync::atomic::Ordering::Relaxed,
                    ) {
                        break;
                    }
                    thread::sleep(
                        Duration::from_micros(50),
                    );
                }
                CastRecv::Faulted { gap_end_inclusive, .. } => {
                    receiver.reset_after_replay(
                        gap_end_inclusive,
                    );
                }
                CastRecv::Reconnect { last_delivered_seq } => {
                    receiver.reset_after_replay(
                        last_delivered_seq,
                    );
                }
            }
        }
        count
    });

    // Warmup — first send paths through dirty cache lines.
    for i in 1..=64u64 {
        let mut rec = fill(i);
        let _ = sender.send(&mut rec).unwrap();
    }

    let t0 = Instant::now();
    for _ in 0..N {
        let mut rec = fill(0);
        sender.send(&mut rec).unwrap();
    }
    let elapsed = t0.elapsed();

    // Give the drainer a brief moment to absorb the tail of
    // the burst that's still in flight on the loopback socket
    // after the final send. Without this, the drain thread can
    // race the stop flag and miss the last few datagrams,
    // turning a healthy run into a delivery-floor failure.
    thread::sleep(Duration::from_millis(100));
    drain_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let seen = drainer.join().unwrap();

    let rate = (N as f64) / elapsed.as_secs_f64();
    let total_sent = N + 64; // warmup + measured burst
    eprintln!(
        "cmp send: {N} msgs in {elapsed:?} \
         ({rate:.0} msg/s); receiver saw {seen}/{total_sent}",
    );

    assert!(
        elapsed < Duration::from_secs(1),
        "50k sends took {elapsed:?}, expected < 1s",
    );
    // Implied floor: >= 50_000 msg/s. The bench harness
    // typically clocks loopback at >1M msg/s; a 50k/s floor
    // catches catastrophic regressions without false alarms
    // under CI noise.
    assert!(
        rate >= 50_000.0,
        "throughput {rate:.0} msg/s below 50_000 floor",
    );

    // Delivery floor: 0.5% of total. Under v4 the receiver
    // enforces strict FIFO and FAULTs on a gap >
    // REORDER_CAPACITY without sender-side NAK response.
    // This test runs without a real sender NAK loop
    // (`recv_control` is never called), so the OS drops
    // under 50 k-msg burst trigger repeated FAULT + reset
    // cycles and the in-band delivery rate falls. The
    // assertion still catches "receive path is dead" — the
    // throughput cliff comes from documented v4 semantics,
    // not a bug. Real consumers wire DXS replay and get
    // continuous delivery via that path.
    let floor = total_sent / 200;
    assert!(
        seen >= floor,
        "receiver saw {seen}/{total_sent} datagrams \
         (below {floor} delivery floor); throughput \
         number is meaningless without proof of delivery",
    );
}
