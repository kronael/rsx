use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::as_bytes;
use rsx_dxs::compute_crc32;
use rsx_dxs::encode_record;
use rsx_dxs::CmpHeartbeat;
use rsx_dxs::Nak;
use rsx_dxs::StatusMessage;
use rsx_dxs::RECORD_HEARTBEAT;
use rsx_dxs::RECORD_NAK;
use rsx_dxs::RECORD_STATUS_MESSAGE;
use std::collections::BTreeMap;

fn make_status_message() -> StatusMessage {
    StatusMessage {
        consumption_seq: 42,
        receiver_window: 512,
        _pad1: [0u8; 48],
    }
}

fn make_nak() -> Nak {
    Nak {
        from_seq: 10,
        count: 5,
        _pad1: [0u8; 48],
    }
}

fn make_heartbeat() -> CmpHeartbeat {
    CmpHeartbeat {
        highest_seq: 999,
        _pad1: [0u8; 56],
    }
}

/// Target: <50ns
fn bench_status_message_encode(c: &mut Criterion) {
    let msg = make_status_message();
    c.bench_function(
        "status_message_encode",
        |b| {
            b.iter(|| {
                let bytes = as_bytes(black_box(&msg));
                let crc = compute_crc32(bytes);
                black_box(encode_record(
                    RECORD_STATUS_MESSAGE,
                    bytes,
                ));
                black_box(crc);
            })
        },
    );
}

/// Target: <50ns
fn bench_status_message_decode(c: &mut Criterion) {
    let msg = make_status_message();
    let bytes = as_bytes(&msg);
    let encoded = encode_record(
        RECORD_STATUS_MESSAGE,
        bytes,
    );
    let payload = &encoded[16..]; // skip header
    c.bench_function(
        "status_message_decode",
        |b| {
            b.iter(|| {
                let p = black_box(payload);
                let decoded = unsafe {
                    std::ptr::read_unaligned(
                        p.as_ptr()
                            as *const StatusMessage,
                    )
                };
                black_box(decoded);
            })
        },
    );
}

/// Target: <50ns
fn bench_nak_encode(c: &mut Criterion) {
    let nak = make_nak();
    c.bench_function("nak_encode", |b| {
        b.iter(|| {
            let bytes = as_bytes(black_box(&nak));
            let crc = compute_crc32(bytes);
            black_box(encode_record(
                RECORD_NAK,
                bytes,
            ));
            black_box(crc);
        })
    });
}

/// Target: <50ns
fn bench_nak_decode(c: &mut Criterion) {
    let nak = make_nak();
    let bytes = as_bytes(&nak);
    let encoded =
        encode_record(RECORD_NAK, bytes);
    let payload = &encoded[16..];
    c.bench_function("nak_decode", |b| {
        b.iter(|| {
            let p = black_box(payload);
            let decoded = unsafe {
                std::ptr::read_unaligned(
                    p.as_ptr() as *const Nak,
                )
            };
            black_box(decoded);
        })
    });
}

/// Target: <50ns
fn bench_heartbeat_encode(c: &mut Criterion) {
    let hb = make_heartbeat();
    c.bench_function("heartbeat_encode", |b| {
        b.iter(|| {
            let bytes = as_bytes(black_box(&hb));
            let crc = compute_crc32(bytes);
            black_box(encode_record(
                RECORD_HEARTBEAT,
                bytes,
            ));
            black_box(crc);
        })
    });
}

/// Target: <50ns
fn bench_heartbeat_decode(c: &mut Criterion) {
    let hb = make_heartbeat();
    let bytes = as_bytes(&hb);
    let encoded =
        encode_record(RECORD_HEARTBEAT, bytes);
    let payload = &encoded[16..];
    c.bench_function("heartbeat_decode", |b| {
        b.iter(|| {
            let p = black_box(payload);
            let decoded = unsafe {
                std::ptr::read_unaligned(
                    p.as_ptr()
                        as *const CmpHeartbeat,
                )
            };
            black_box(decoded);
        })
    });
}

/// Target: <100ns
/// Reorder buffer is BTreeMap<u64, Vec<u8>> inside
/// CmpReceiver. Bench standalone insert + lookup.
fn bench_reorder_buf_insert_lookup(
    c: &mut Criterion,
) {
    c.bench_function(
        "reorder_buf_insert_lookup",
        |b| {
            b.iter(|| {
                let mut buf: BTreeMap<u64, Vec<u8>> =
                    BTreeMap::new();
                let data = vec![0u8; 80];
                buf.insert(
                    black_box(42),
                    data,
                );
                let entry =
                    buf.first_entry();
                black_box(entry);
            })
        },
    );
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
    bench_status_message_encode,
    bench_status_message_decode,
    bench_nak_encode,
    bench_nak_decode,
    bench_heartbeat_encode,
    bench_heartbeat_decode,
    bench_reorder_buf_insert_lookup,
);
criterion_main!(benches);
