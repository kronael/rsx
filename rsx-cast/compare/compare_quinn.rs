//! Quinn (QUIC) loopback round-trip — `compare/quinn.md` companion bench.
//!
//! What this measures
//! ------------------
//! Application-visible loopback RTT for QUIC streams via Quinn,
//! plus a TCP `TCP_NODELAY` baseline for context. Payload size
//! matched to `cmp_rtt_bench.rs` (128 B — `FillRecord` is 128 B
//! per `rsx-messages/src/lib.rs:78`).
//!
//! Three scenarios:
//!
//! `quinn_rtt_new_stream_128b` — open a fresh bidirectional stream
//!   every iteration; client `write_all` + `finish`, server
//!   `read_to_end` and echoes back. Measures the "per-RPC" cost:
//!   QUIC's intended unit of work for HTTP/3 etc.
//!
//! `quinn_rtt_persistent_128b` — open one bidirectional stream in
//!   setup, reuse it across iterations with a fixed 128 B record
//!   read via `read_exact`. Measures steady-state QUIC overhead
//!   with stream creation AND TLS handshake outside the timed loop.
//!
//! `tcp_rtt_nodelay_128b` — Tokio TCP with `set_nodelay(true)`,
//!   persistent connection, same 128 B `read_exact` framing.
//!
//! Client (Criterion timer) pinned to core 2. The single-threaded
//! Tokio runtime + server tasks run on the same thread (current_thread
//! runtime, no work-stealing), so they share core 2 with the client.
//! For QUIC/TCP this is fine because the timed iteration consists of
//! `rt.block_on(...)` which drives both sides before returning. We do
//! NOT pin to a second core — there is no second OS thread to pin.
//!
//! What is NOT measured
//! --------------------
//! - TLS handshake (excluded by design; happens once in
//!   `make_connected_pair`, ~150–400 µs on loopback).
//! - Connection migration / packet reordering.
//! - WAN behaviour (loopback only).
//! - Multi-stream throughput (single-stream per bench).
//!
//! Apples-to-apples with `cmp_rtt_bench.rs`
//! ----------------------------------------
//! Same payload size (128 B). Same Criterion sample_size (50).
//! Same loopback (127.0.0.1). One important asymmetry remains:
//!
//!   `rt.block_on()` is on the timed critical path of every QUIC
//!   and TCP iteration. This adds Tokio executor / waker
//!   scheduling overhead (~hundreds of ns) that CMP does NOT pay
//!   — CMP's RTT bench is synchronous and spin-polls. This is
//!   fundamental to Quinn's async API surface and cannot be
//!   eliminated without forking the crate. The published Quinn
//!   numbers in `compare/quinn.md` (picoquic 20 µs min, iggy
//!   ~1.97 ms avg) include the same overhead — so the
//!   comparison is fair against published Quinn data, even
//!   though it is biased upward against CMP's syscall-only path.
//!
//! Loss simulation (separate run, requires root):
//!   sudo tc qdisc add dev lo root netem loss 0.1%
//!   cargo bench -p rsx-cast --bench compare_quinn
//!   sudo tc qdisc del dev lo root
//! The bench itself does not require root or tc.

use core_affinity::CoreId;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use quinn::ClientConfig;
use quinn::Endpoint;
use quinn::RecvStream;
use quinn::SendStream;
use quinn::ServerConfig;
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::runtime::Builder;

// Match FillRecord (128 B, rsx-messages/src/lib.rs:78) so the
// QUIC/TCP numbers are size-comparable to cmp_rtt_bench.rs.
const PAYLOAD_LEN: usize = 128;

/// Pin this thread (the only OS thread used by these single-threaded
/// runtime benches) to core 2 so it doesn't migrate.
fn pin_self() {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let core = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    core_affinity::set_for_current(core);
}

fn make_connected_pair(
    rt: &tokio::runtime::Runtime,
) -> (Endpoint, Endpoint, quinn::Connection, quinn::Connection) {
    rt.block_on(async {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .expect("rcgen self-signed");
        let cert_der = cert.cert.der().clone();
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

        let srv_cfg = ServerConfig::with_single_cert(
            vec![cert_der.clone()],
            key_der.into(),
        ).expect("server config");
        let server_ep = Endpoint::server(
            srv_cfg,
            "127.0.0.1:0".parse::<SocketAddr>().expect("srv addr parse"),
        ).expect("server endpoint");
        let srv_addr = server_ep.local_addr().expect("server local_addr");

        let server_ep2 = server_ep.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Some(inc) = server_ep2.accept().await {
                if let Ok(conn) = inc.await {
                    tx.send(conn).expect("send accepted conn");
                }
            }
        });

        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert_der).expect("roots.add cert");
        let cli_cfg = ClientConfig::with_root_certificates(Arc::new(roots))
            .expect("client config");
        let mut client_ep = Endpoint::client(
            "127.0.0.1:0".parse::<SocketAddr>().expect("cli addr parse"),
        ).expect("client endpoint");
        client_ep.set_default_client_config(cli_cfg);
        let cli_conn = client_ep
            .connect(srv_addr, "localhost")
            .expect("connect call")
            .await
            .expect("connect await");
        let srv_conn = rx.await.expect("server accept");

        (server_ep, client_ep, cli_conn, srv_conn)
    })
}

// ── QUIC: new stream per iteration ───────────────────────────────────────────

fn bench_quinn_new_stream(c: &mut Criterion) {
    pin_self();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = Builder::new_current_thread().enable_all().build()
        .expect("tokio runtime");
    let (server_ep, client_ep, cli_conn, srv_conn) = make_connected_pair(&rt);

    rt.spawn(async move {
        while let Ok((mut send, mut recv)) = srv_conn.accept_bi().await {
            tokio::spawn(async move {
                // Client `finish()`es the stream after sending PAYLOAD_LEN
                // bytes, so read_to_end is bounded by the stream FIN.
                let buf = recv.read_to_end(PAYLOAD_LEN).await
                    .expect("server read_to_end");
                assert_eq!(buf.len(), PAYLOAD_LEN, "server got partial payload");
                send.write_all(&buf).await.expect("server write_all");
                send.finish().expect("server finish");
            });
        }
    });

    let payload = [0xAAu8; PAYLOAD_LEN];
    let mut recv_buf = vec![0u8; PAYLOAD_LEN];

    // Readiness barrier: complete one full RTT before timing.
    rt.block_on(async {
        let (mut send, mut recv) = cli_conn.open_bi().await
            .expect("warmup open_bi");
        send.write_all(&payload).await.expect("warmup write");
        send.finish().expect("warmup finish");
        let echo = recv.read_to_end(PAYLOAD_LEN).await
            .expect("warmup read_to_end");
        assert_eq!(echo.len(), PAYLOAD_LEN, "warmup got partial echo");
    });

    c.bench_function("quinn_rtt_new_stream_128b", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (mut send, mut recv) = cli_conn.open_bi().await
                    .expect("open_bi");
                send.write_all(black_box(&payload)).await
                    .expect("client write_all");
                send.finish().expect("client finish");
                let echo = recv.read_to_end(PAYLOAD_LEN).await
                    .expect("client read_to_end");
                assert_eq!(echo.len(), PAYLOAD_LEN, "partial echo");
                recv_buf.copy_from_slice(&echo);
                black_box(&recv_buf);
            });
        });
    });

    rt.block_on(async {
        cli_conn.close(0u32.into(), b"done");
        client_ep.wait_idle().await;
        server_ep.wait_idle().await;
    });
}

// ── QUIC: persistent stream ──────────────────────────────────────────────────

async fn read_exact(recv: &mut RecvStream, buf: &mut [u8]) {
    recv.read_exact(buf).await.expect("read_exact on persistent stream");
}

async fn write_payload(send: &mut SendStream, data: &[u8]) {
    send.write_all(data).await.expect("write_all on persistent stream");
}

fn bench_quinn_persistent(c: &mut Criterion) {
    pin_self();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = Builder::new_current_thread().enable_all().build()
        .expect("tokio runtime");
    let (server_ep, client_ep, cli_conn, srv_conn) = make_connected_pair(&rt);

    // Spawn the server echo task first; it accepts the bidi stream
    // once the client opens one and writes the first byte.
    rt.spawn(async move {
        let (mut s, mut r) = srv_conn.accept_bi().await
            .expect("server accept_bi on persistent stream");
        let mut buf = [0u8; PAYLOAD_LEN];
        loop {
            if r.read_exact(&mut buf).await.is_err() {
                break;
            }
            if s.write_all(&buf).await.is_err() {
                break;
            }
        }
    });

    let (mut cli_send, mut cli_recv) = rt.block_on(async {
        cli_conn.open_bi().await.expect("client open_bi")
    });

    let payload = [0xAAu8; PAYLOAD_LEN];
    let mut recv_buf = [0u8; PAYLOAD_LEN];

    // Readiness barrier: a complete RTT before timing begins. The
    // first write is what causes accept_bi to fire on the server.
    rt.block_on(async {
        write_payload(&mut cli_send, &payload).await;
        read_exact(&mut cli_recv, &mut recv_buf).await;
    });

    c.bench_function("quinn_rtt_persistent_128b", |b| {
        b.iter(|| {
            rt.block_on(async {
                write_payload(&mut cli_send, black_box(&payload)).await;
                read_exact(&mut cli_recv, &mut recv_buf).await;
                black_box(&recv_buf);
            });
        });
    });

    rt.block_on(async {
        cli_conn.close(0u32.into(), b"done");
        client_ep.wait_idle().await;
        server_ep.wait_idle().await;
    });
}

// ── TCP nodelay baseline ─────────────────────────────────────────────────────

fn bench_tcp_rtt(c: &mut Criterion) {
    pin_self();
    let rt = Builder::new_current_thread().enable_all().build()
        .expect("tokio runtime");

    let mut stream = rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await
            .expect("listener bind");
        let addr = listener.local_addr().expect("listener addr");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                sock.set_nodelay(true).expect("server set_nodelay");
                tokio::spawn(async move {
                    let mut buf = [0u8; PAYLOAD_LEN];
                    loop {
                        if sock.read_exact(&mut buf).await.is_err() {
                            break;
                        }
                        if sock.write_all(&buf).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        let s = TcpStream::connect(addr).await.expect("tcp connect");
        s.set_nodelay(true).expect("client set_nodelay");
        s
    });

    let payload = [0xAAu8; PAYLOAD_LEN];
    let mut recv_buf = [0u8; PAYLOAD_LEN];

    // Readiness barrier: complete one RTT before timing begins.
    rt.block_on(async {
        stream.write_all(&payload).await.expect("warmup write");
        stream.read_exact(&mut recv_buf).await.expect("warmup read");
    });

    c.bench_function("tcp_rtt_nodelay_128b", |b| {
        b.iter(|| {
            rt.block_on(async {
                stream.write_all(black_box(&payload)).await
                    .expect("client write_all");
                stream.read_exact(&mut recv_buf).await
                    .expect("client read_exact");
                black_box(&recv_buf);
            });
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_quinn_new_stream, bench_quinn_persistent, bench_tcp_rtt
}
criterion_main!(benches);
