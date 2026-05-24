//! SoupBinTCP loopback round-trip comparison.
//!
//! What this measures
//! -----------------
//! Clean-room implementation of just enough Nasdaq SoupBinTCP
//! framing to round-trip one 64-byte payload over std
//! `TcpStream` (loopback, `TCP_NODELAY` on both ends, persistent
//! connection — handshake is outside the timed loop).
//!
//! Wire format per packet (big-endian):
//!   0..2    length (u16): bytes that follow, i.e. type + payload
//!   2       packet_type (u8 ASCII)
//!   3..     payload
//!
//! Bench packets use type `U` (client → server, unsequenced) for
//! the inbound direction and `S` (server → client, sequenced) for
//! the echo direction — matching real OUCH order entry on the way
//! in and ITCH/sequenced data on the way out. Both sides parse a
//! 3-byte header and then the payload (full SoupBin parse + emit,
//! not raw byte echo).
//!
//! Payload: 64 bytes. Total wire per direction: 2 + 1 + 64 = 67 B.
//!
//! Compare with:
//!   udp_rtt_bench       raw UDP floor (~2 µs)
//!   cmp_rtt_bench       CMP NAK overhead (~10 µs)
//!   compare_moldudp64   Nasdaq's UDP framing (~3–6 µs)
//!   compare_quinn       QUIC overhead (~200–500 µs)
//!
//! See compare/soupbintcp.md for protocol details.
//!
//! TODO(pinning): a parallel sub is adding core_affinity across
//! the bench suite — this thread spawn picks up pinning in the
//! follow-up merge.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

/// SoupBinTCP common header: 2-byte length + 1-byte type.
const SOUP_HDR: usize = 3;
/// Bench payload size matches every other compare_* harness.
const PAYLOAD: usize = 64;
/// Total wire = 2 (length) + 1 (type) + 64 (payload).
const WIRE_BYTES: usize = SOUP_HDR + PAYLOAD;
/// SoupBin `U` packet: client → server, unsequenced data (OUCH-side).
const PKT_UNSEQUENCED: u8 = b'U';
/// SoupBin `S` packet: server → client, sequenced data (ITCH-side).
const PKT_SEQUENCED: u8 = b'S';
/// SoupBin `Z` packet: end of session, used here to tell the
/// echoer thread to exit cleanly.
const PKT_END_OF_SESSION: u8 = b'Z';

/// Frame a SoupBinTCP packet into `buf`. Returns total written.
fn frame_packet(buf: &mut [u8], pkt_type: u8, payload: &[u8]) -> usize {
    // length covers packet_type + payload, NOT the length field itself.
    let len = (1 + payload.len()) as u16;
    buf[0..2].copy_from_slice(&len.to_be_bytes());
    buf[2] = pkt_type;
    buf[SOUP_HDR..SOUP_HDR + payload.len()].copy_from_slice(payload);
    SOUP_HDR + payload.len()
}

/// Read exactly N bytes from a TcpStream, spinning on WouldBlock.
/// Asserts on connection close or hard errors — bench setup should
/// not see them.
fn read_exact_spin(s: &mut TcpStream, buf: &mut [u8]) {
    let mut off = 0;
    while off < buf.len() {
        match s.read(&mut buf[off..]) {
            Ok(0) => panic!("SoupBin peer closed mid-frame"),
            Ok(n) => off += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::hint::spin_loop();
            }
            Err(e) => panic!("SoupBin read: {e}"),
        }
    }
}

/// Write all N bytes to a TcpStream, spinning on WouldBlock. Non-
/// blocking `TcpStream::write_all` can return `WouldBlock` or fail
/// after a partial write under TCP backpressure; this helper retries
/// until the kernel accepts every byte, mirroring `read_exact_spin`.
fn write_all_spin(s: &mut TcpStream, buf: &[u8]) {
    let mut off = 0;
    while off < buf.len() {
        match s.write(&buf[off..]) {
            Ok(0) => panic!("SoupBin peer closed mid-write"),
            Ok(n) => off += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::hint::spin_loop();
            }
            Err(e) => panic!("SoupBin write: {e}"),
        }
    }
}

/// Parse a SoupBin header from a 3-byte buffer. Returns
/// (payload_length, packet_type). Payload length is the
/// announced length minus 1 (because length includes type byte).
fn parse_header(hdr: &[u8; SOUP_HDR]) -> (usize, u8) {
    let len = u16::from_be_bytes([hdr[0], hdr[1]]) as usize;
    assert!(len >= 1, "SoupBin length must include type byte");
    (len - 1, hdr[2])
}

fn bench_soupbintcp_rtt(c: &mut Criterion) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let srv_addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    // Echoer thread: accept one connection, then loop on
    // (read header → read payload → frame echo → write echo).
    let handle = thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        sock.set_nodelay(true).expect("server nodelay");
        sock.set_nonblocking(true).expect("server nonblocking");

        let mut hdr = [0u8; SOUP_HDR];
        let mut payload = [0u8; PAYLOAD];
        let mut tx = [0u8; WIRE_BYTES];

        while !stop_clone.load(Ordering::Relaxed) {
            read_exact_spin(&mut sock, &mut hdr);
            let (payload_len, pkt_type) = parse_header(&hdr);
            if pkt_type == PKT_END_OF_SESSION {
                return;
            }
            assert_eq!(
                payload_len, PAYLOAD,
                "echoer expected {PAYLOAD}-byte payload, got {payload_len}",
            );
            assert_eq!(pkt_type, PKT_UNSEQUENCED, "echoer expected 'U'");
            read_exact_spin(&mut sock, &mut payload);
            // Echo back as a sequenced ('S') data packet — full
            // frame on the echo side, not raw byte mirror.
            let n = frame_packet(&mut tx, PKT_SEQUENCED, &payload);
            assert_eq!(n, WIRE_BYTES);
            write_all_spin(&mut sock, &tx[..n]);
        }
    });

    let mut pinger = TcpStream::connect(srv_addr).unwrap();
    pinger.set_nodelay(true).unwrap();
    pinger.set_nonblocking(true).unwrap();

    let payload = [0xAAu8; PAYLOAD];
    let mut tx = [0u8; WIRE_BYTES];
    let mut rx_hdr = [0u8; SOUP_HDR];
    let mut rx_payload = [0u8; PAYLOAD];

    c.bench_function("soupbintcp_rtt_loopback_64b", |b| {
        b.iter(|| {
            let n = frame_packet(
                &mut tx,
                PKT_UNSEQUENCED,
                black_box(&payload),
            );
            assert_eq!(n, WIRE_BYTES);
            write_all_spin(&mut pinger, &tx[..n]);
            read_exact_spin(&mut pinger, &mut rx_hdr);
            let (payload_len, pkt_type) = parse_header(&rx_hdr);
            assert_eq!(payload_len, PAYLOAD, "echoed payload length");
            assert_eq!(pkt_type, PKT_SEQUENCED, "echoed packet type");
            read_exact_spin(&mut pinger, &mut rx_payload);
            black_box(&rx_payload);
        });
    });

    // Tell the echoer to exit via an end-of-session ('Z') packet
    // (header only, no payload).
    stop.store(true, Ordering::Release);
    let mut shutdown_buf = [0u8; SOUP_HDR];
    let n = frame_packet(&mut shutdown_buf, PKT_END_OF_SESSION, &[]);
    write_all_spin(&mut pinger, &shutdown_buf[..n]);
    let _ = handle.join();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_soupbintcp_rtt
}
criterion_main!(benches);
