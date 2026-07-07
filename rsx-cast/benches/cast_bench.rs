//! casting protocol-record encode/decode (NAK, Heartbeat). Wire-level primitives; not on the per-packet send path.
//!
//! Worker thread pinned to core 2 for measurement stability.
//!
//! See `docs/benches.md` for the full bench index +
//! production-leg attribution.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_cast::as_bytes;
use rsx_cast::compute_crc32;
use rsx_cast::encode_record;
use rsx_cast::CastHeartbeat;
use rsx_cast::Nak;
use rsx_cast::RECORD_HEARTBEAT;
use rsx_cast::RECORD_NAK;
use std::collections::BTreeMap;

fn pin_worker() {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let core = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    core_affinity::set_for_current(core);
}

fn make_nak() -> Nak {
    Nak {
        from_seq: 10,
        count: 5,
        _pad1: [0u8; 48],
    }
}

fn make_heartbeat() -> CastHeartbeat {
    CastHeartbeat {
        highest_seq: 999,
        _pad1: [0u8; 56],
    }
}

/// Target: <50ns
fn bench_nak_encode(c: &mut Criterion) {
    pin_worker();
    let nak = make_nak();
    c.bench_function("nak_encode", |b| {
        b.iter(|| {
            let bytes = as_bytes(black_box(&nak));
            let crc = compute_crc32(bytes);
            black_box(encode_record(RECORD_NAK, bytes));
            black_box(crc);
        })
    });
}

/// Target: <50ns
fn bench_nak_decode(c: &mut Criterion) {
    pin_worker();
    let nak = make_nak();
    let bytes = as_bytes(&nak);
    let encoded = encode_record(RECORD_NAK, bytes);
    let payload = &encoded[16..];
    c.bench_function("nak_decode", |b| {
        b.iter(|| {
            let p = black_box(payload);
            let decoded = unsafe { std::ptr::read_unaligned(p.as_ptr() as *const Nak) };
            black_box(decoded);
        })
    });
}

/// Target: <50ns
fn bench_heartbeat_encode(c: &mut Criterion) {
    pin_worker();
    let hb = make_heartbeat();
    c.bench_function("heartbeat_encode", |b| {
        b.iter(|| {
            let bytes = as_bytes(black_box(&hb));
            let crc = compute_crc32(bytes);
            black_box(encode_record(RECORD_HEARTBEAT, bytes));
            black_box(crc);
        })
    });
}

/// Target: <50ns
fn bench_heartbeat_decode(c: &mut Criterion) {
    pin_worker();
    let hb = make_heartbeat();
    let bytes = as_bytes(&hb);
    let encoded = encode_record(RECORD_HEARTBEAT, bytes);
    let payload = &encoded[16..];
    c.bench_function("heartbeat_decode", |b| {
        b.iter(|| {
            let p = black_box(payload);
            let decoded = unsafe { std::ptr::read_unaligned(p.as_ptr() as *const CastHeartbeat) };
            black_box(decoded);
        })
    });
}

/// Target: <100ns
/// Reorder buffer is BTreeMap<u64, Vec<u8>> inside
/// CastReceiver. Bench standalone insert + lookup. Map
/// allocated ONCE outside the timed closure; the inner
/// loop swaps a pre-allocated Vec<u8> in and out so the
/// per-iteration cost is BTreeMap ops only (no Vec alloc,
/// no map alloc). Previous version reallocated both per
/// iteration which buried the BTreeMap cost we want.
fn bench_reorder_buf_insert_lookup(c: &mut Criterion) {
    pin_worker();
    let mut buf: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    let mut stash: Vec<u8> = vec![0u8; 80];
    let mut key: u64 = 0;
    c.bench_function("reorder_buf_insert_lookup", |b| {
        b.iter(|| {
            key = key.wrapping_add(1);
            // Move the pre-allocated Vec into the map.
            let payload = std::mem::take(&mut stash);
            buf.insert(black_box(key), payload);
            let entry = buf.first_entry();
            black_box(&entry);
            // Reclaim the Vec so the next iter doesn't alloc.
            if let Some(v) = buf.remove(&key) {
                stash = v;
            }
        })
    });
}

// Network-dependent benchmarks (skipped):
// - bench_cmp_send_udp_loopback: needs real UDP
//   sockets, target <10us RTT
// - bench_cmp_send_recv_1m_sustained: needs real
//   sender+receiver, target >1M msg/s
// - bench_tcp_replay_100k_records: needs TCP server,
//   target <1s
// - bench_tcp_sustained_throughput: needs TCP live
//   tail, target >500K msg/s
// - bench_gap_detect_to_retransmit: needs full
//   sender+receiver+WAL, target <50us
// - bench_nak_to_recovery_latency: needs full
//   pipeline, target <100us
// - bench_flow_control_stall_resume: needs sender
//   +receiver, target <1ms
// - bench_zero_alloc_send_recv_loop: needs counting
//   allocator + full pipeline, target 0 heap

criterion_group!(
    benches,
    bench_nak_encode,
    bench_nak_decode,
    bench_heartbeat_encode,
    bench_heartbeat_decode,
    bench_reorder_buf_insert_lookup,
);
criterion_main!(benches);
