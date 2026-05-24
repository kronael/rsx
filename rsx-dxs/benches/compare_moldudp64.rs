//! MoldUDP64 loopback round-trip comparison.
//!
//! What this measures
//! -----------------
//! Clean-room implementation of just enough Nasdaq MoldUDP64
//! framing to round-trip one 64-byte payload over **unicast**
//! `std::net::UdpSocket` on 127.0.0.1. Both directions perform
//! a full parse + emit of the 20-byte downstream header plus
//! one length-prefixed message.
//!
//! Wire format per packet (big-endian):
//!   0..10   session id (ASCII, space-padded)
//!   10..18  sequence number (u64)
//!   18..20  message count (u16)
//!   20..22  message 0 length (u16)
//!   22..86  message 0 payload (64 bytes)
//!
//! Total wire = 86 bytes per direction.
//!
//! We bench unicast, not multicast: loopback multicast on Linux
//! adds IGMP / IP_ADD_MEMBERSHIP / route-hint plumbing that
//! would measure the kernel multicast path, not MoldUDP64's
//! framing cost. Unicast keeps this apples-to-apples with
//! `udp_rtt_bench`, `compare_kcp`, and `compare_tcp`.
//!
//! Compare with:
//!   udp_rtt_bench       raw UDP floor (~2 µs)
//!   cmp_rtt_bench       CMP NAK overhead (~10 µs)
//!   compare_soupbintcp  Nasdaq's TCP framing (~10–30 µs)
//!
//! See compare/moldudp64.md for protocol details.
//!
//! TODO(pinning): a parallel sub is adding core_affinity across
//! the bench suite — this thread spawn picks up pinning in the
//! follow-up merge.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

/// MoldUDP64 downstream header is fixed at 20 bytes.
const MOLD_HDR: usize = 20;
/// Per-message length prefix.
const MSG_LEN_PREFIX: usize = 2;
/// Bench payload size matches every other compare_* harness.
const PAYLOAD: usize = 64;
/// One packet, one message: 20 + 2 + 64 = 86 bytes.
const WIRE_BYTES: usize = MOLD_HDR + MSG_LEN_PREFIX + PAYLOAD;
/// Receive buffer ceiling. Generous so a malformed peer can't
/// overflow the parse path during the bench.
const RECV_BUF: usize = 256;
/// Sentinel sequence used by the pinger to ask the echoer to
/// exit cleanly. msg_count = 0xFFFF is "end of session" in the
/// MoldUDP64 spec; reusing it here as our shutdown signal.
const END_OF_SESSION: u16 = 0xFFFF;

/// Pack a MoldUDP64 downstream packet carrying exactly one
/// 64-byte message. Returns the framed slice length.
fn frame_packet(
    buf: &mut [u8],
    session: &[u8; 10],
    seq: u64,
    msg_count: u16,
    payload: &[u8],
) -> usize {
    buf[0..10].copy_from_slice(session);
    buf[10..18].copy_from_slice(&seq.to_be_bytes());
    buf[18..20].copy_from_slice(&msg_count.to_be_bytes());
    if msg_count == 0 || msg_count == END_OF_SESSION {
        // Heartbeat or end-of-session: header only, no messages.
        return MOLD_HDR;
    }
    // One message, length-prefixed, payload bytes follow.
    let len = payload.len() as u16;
    buf[20..22].copy_from_slice(&len.to_be_bytes());
    buf[22..22 + payload.len()].copy_from_slice(payload);
    MOLD_HDR + MSG_LEN_PREFIX + payload.len()
}

/// Parse a MoldUDP64 packet. Returns (seq, msg_count, payload_slice).
/// Caller must verify session ID externally if it cares.
fn parse_packet(buf: &[u8]) -> (u64, u16, &[u8]) {
    assert!(buf.len() >= MOLD_HDR, "short MoldUDP64 packet");
    let mut seq_bytes = [0u8; 8];
    seq_bytes.copy_from_slice(&buf[10..18]);
    let seq = u64::from_be_bytes(seq_bytes);
    let count = u16::from_be_bytes([buf[18], buf[19]]);
    if count == 0 || count == END_OF_SESSION {
        return (seq, count, &[]);
    }
    assert!(
        buf.len() >= MOLD_HDR + MSG_LEN_PREFIX,
        "MoldUDP64 packet truncated before message length",
    );
    let msg_len = u16::from_be_bytes([buf[20], buf[21]]) as usize;
    let end = MOLD_HDR + MSG_LEN_PREFIX + msg_len;
    assert!(buf.len() >= end, "MoldUDP64 packet truncated before payload");
    (seq, count, &buf[MOLD_HDR + MSG_LEN_PREFIX..end])
}

fn bench_moldudp64_rtt(c: &mut Criterion) {
    let echoer = UdpSocket::bind("127.0.0.1:0").unwrap();
    let echoer_addr = echoer.local_addr().unwrap();
    let pinger = UdpSocket::bind("127.0.0.1:0").unwrap();

    echoer.set_nonblocking(true).unwrap();
    pinger.set_nonblocking(true).unwrap();

    // Spaces-padded ASCII session ID, MoldUDP64 convention.
    let session: [u8; 10] = *b"RSXBENCH01";

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    // Echoer thread: parse every packet, build a fresh
    // MoldUDP64 packet back with the echoer's own seq counter
    // so we measure full framing on both sides.
    let handle = thread::spawn(move || {
        let mut rx = [0u8; RECV_BUF];
        let mut tx = [0u8; RECV_BUF];
        let mut echo_seq: u64 = 1;
        while !stop_clone.load(Ordering::Relaxed) {
            match echoer.recv_from(&mut rx) {
                Ok((n, src)) => {
                    let (_seq, count, payload) = parse_packet(&rx[..n]);
                    if count == END_OF_SESSION {
                        return;
                    }
                    if count == 0 {
                        // Heartbeat: bounce a heartbeat back. The
                        // seq in a heartbeat is the next expected
                        // message sequence, so we report `echo_seq`
                        // (what we'd assign to the NEXT data packet)
                        // and do NOT bump it — heartbeats consume no
                        // message sequence per spec.
                        let len = frame_packet(
                            &mut tx, &session, echo_seq, 0, &[],
                        );
                        echoer
                            .send_to(&tx[..len], src)
                            .expect("echoer heartbeat send");
                        continue;
                    }
                    let len = frame_packet(
                        &mut tx, &session, echo_seq, 1, payload,
                    );
                    assert_eq!(len, WIRE_BYTES, "echoer wire size");
                    echoer
                        .send_to(&tx[..len], src)
                        .expect("echoer data send");
                    echo_seq += 1;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::hint::spin_loop();
                }
                Err(e) => panic!("echoer recv: {e}"),
            }
        }
    });

    let payload = [0xAAu8; PAYLOAD];
    let mut tx = [0u8; RECV_BUF];
    let mut rx = [0u8; RECV_BUF];
    let mut ping_seq: u64 = 1;

    c.bench_function("moldudp64_rtt_loopback_64b", |b| {
        b.iter(|| {
            let len = frame_packet(
                &mut tx,
                &session,
                ping_seq,
                1,
                black_box(&payload),
            );
            assert_eq!(len, WIRE_BYTES, "pinger wire size");
            pinger
                .send_to(&tx[..len], echoer_addr)
                .expect("pinger send");
            // Spin until the echoed packet arrives.
            loop {
                match pinger.recv_from(&mut rx) {
                    Ok((n, _)) => {
                        let (_seq, count, echo_payload) =
                            parse_packet(&rx[..n]);
                        assert_eq!(count, 1, "echoed msg_count");
                        assert_eq!(echo_payload.len(), PAYLOAD);
                        black_box(echo_payload);
                        break;
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        std::hint::spin_loop();
                    }
                    Err(e) => panic!("pinger recv: {e}"),
                }
            }
            ping_seq += 1;
        });
    });

    // Tell the echoer to exit (end-of-session packet, header only).
    stop.store(true, Ordering::Release);
    let mut shutdown_buf = [0u8; MOLD_HDR];
    let n = frame_packet(
        &mut shutdown_buf,
        &session,
        ping_seq,
        END_OF_SESSION,
        &[],
    );
    let _ = pinger.send_to(&shutdown_buf[..n], echoer_addr);
    let _ = handle.join();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_moldudp64_rtt
}
criterion_main!(benches);
