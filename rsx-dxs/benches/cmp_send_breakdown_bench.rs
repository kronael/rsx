//! Sub-stage breakdown of `CmpSender::send` body.
//!
//! `bench-match-rt` showed the gateway-side
//! `CmpSender::send` takes ~3.8 µs p50 — the dominant cost
//! in the 9.6 µs in-process matching round-trip. This
//! bench attributes that 3.8 µs to specific sub-steps so
//! we know what's worth optimising.
//!
//! Each sub-step is benched in isolation, replicating what
//! `cmp.rs:185-237` does internally. Adds up: the sum
//! should be ≈ the bench-match-rt `gw_send` 3 767 ns p50.
//!
//! Worker thread (the Criterion timing thread) pinned to core 2.
//! The `sendto_144b_loopback` bench's drain thread pinned to core 3.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::encode_utils::compute_crc32;
use rsx_dxs::header::WalHeader;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::mem;
use std::net::UdpSocket;

const RECORD_FILL: u16 = 5; // RECORD_FILL constant in rsx-messages

/// Cores 2 + 3 (sender/echoer convention). Falls back to 0/1 if
/// fewer than 4 cores are available.
fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let w = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let d = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (w, d)
}

fn pin_worker() {
    let (w, _) = pick_cores();
    core_affinity::set_for_current(w);
}

fn fill_record() -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 100,
        maker_order_id_hi: 0,
        maker_order_id_lo: 200,
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

fn as_bytes<T>(val: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            val as *const T as *const u8,
            mem::size_of::<T>(),
        )
    }
}

/// Sub-step 1: CRC32 over a 128-byte FillRecord payload.
/// `crc32fast` is SIMD-accelerated on x86_64; expect ~30 ns.
fn bench_crc32_128b(c: &mut Criterion) {
    pin_worker();
    let rec = fill_record();
    let payload = as_bytes(&rec);
    c.bench_function("send.crc32_128b", |b| {
        b.iter(|| {
            black_box(compute_crc32(black_box(payload)));
        });
    });
}

/// Sub-step 2: build WalHeader + serialize to bytes.
fn bench_header_build(c: &mut Criterion) {
    pin_worker();
    c.bench_function("send.header_build", |b| {
        b.iter(|| {
            let hdr = WalHeader::new(
                RECORD_FILL,
                128,
                black_box(0xdeadbeefu32),
            );
            black_box(hdr.to_bytes());
        });
    });
}

/// Sub-step 3: two memcpy ops (header → buf, payload → buf).
/// Same shape as cmp.rs:207-210.
fn bench_buf_pack(c: &mut Criterion) {
    pin_worker();
    let rec = fill_record();
    let payload = as_bytes(&rec);
    let header = WalHeader::new(RECORD_FILL, 128, 0);
    let hdr_bytes = header.to_bytes();
    let mut buf = vec![0u8; WalHeader::SIZE + 128];
    c.bench_function("send.buf_pack_144b", |b| {
        b.iter(|| {
            buf[..WalHeader::SIZE]
                .copy_from_slice(black_box(&hdr_bytes));
            buf[WalHeader::SIZE
                ..WalHeader::SIZE + 128]
                .copy_from_slice(black_box(payload));
            black_box(&buf);
        });
    });
}

/// Sub-step 4: sendto syscall over UDP loopback. The
/// receiver is a dedicated socket that drains in a worker
/// thread so the kernel queue never fills. This is where
/// the bulk of the 3.8 µs lives.
fn bench_sendto_loopback(c: &mut Criterion) {
    let (worker_core, drain_core) = pick_cores();
    let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv.local_addr().unwrap();
    let send = UdpSocket::bind("127.0.0.1:0").unwrap();
    // Drain in a thread so the kernel queue doesn't back up.
    // Pin the drain to a different core than the sender so the
    // two contexts don't time-slice the same core (which would
    // serialize the sendto with the drain and inflate latency).
    let stop =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_clone = std::sync::Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        core_affinity::set_for_current(drain_core);
        let mut buf = [0u8; 256];
        recv.set_nonblocking(true).unwrap();
        while !stop_clone.load(
            std::sync::atomic::Ordering::Relaxed,
        ) {
            match recv.recv_from(&mut buf) {
                Ok(_) => {}
                Err(ref e)
                    if e.kind()
                        == std::io::ErrorKind::WouldBlock => {
                    std::hint::spin_loop();
                }
                Err(_) => return,
            }
        }
    });
    core_affinity::set_for_current(worker_core);
    let payload = vec![0xAAu8; 144];
    c.bench_function("send.sendto_144b_loopback", |b| {
        b.iter(|| {
            send.send_to(black_box(&payload), recv_addr)
                .unwrap();
        });
    });
    stop.store(true, std::sync::atomic::Ordering::Release);
    let _ = handle.join();
}

/// Sub-step 5: NAK send-ring slot copy. cmp.rs:223-224 —
/// a memcpy of 128 bytes into a preallocated ring slot. Note: the
/// production code only stages the first SEND_RING_FRAME_BYTES=128
/// of the 144-byte frame; longer headers are marked dirty
/// (cmp.rs:225). So this bench correctly mirrors a 128-byte copy
/// even though the framed payload is 144 bytes on the wire.
fn bench_ring_cache_copy(c: &mut Criterion) {
    pin_worker();
    let mut ring = vec![0u8; 4096 * 128];
    let frame = vec![0xAAu8; 144];
    let mut slot: usize = 0;
    c.bench_function("send.ring_cache_copy_128b", |b| {
        b.iter(|| {
            let off = (slot & 4095) * 128;
            ring[off..off + 128]
                .copy_from_slice(&frame[..128]);
            slot = slot.wrapping_add(1);
            black_box(&ring);
        });
    });
}

criterion_group!(
    benches,
    bench_crc32_128b,
    bench_header_build,
    bench_buf_pack,
    bench_sendto_loopback,
    bench_ring_cache_copy,
);
criterion_main!(benches);
