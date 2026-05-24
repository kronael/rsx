//! Quinn (QUIC) and TCP loopback round-trip comparison.
//!
//! What this measures
//! -----------------
//! - `quinn_rtt_loopback_64b`: QUIC bidirectional stream RTT on loopback.
//! - `tcp_rtt_loopback_64b`: raw TCP stream RTT on loopback.
//!
//! Payload: 64 bytes (matches CMP exchange frame size).
//! TLS handshake and TCP connection setup are NOT in the timed loop.
//!
//! Expected results (from literature):
//!   TCP p50:   ~1 000 µs  (iggy/#606, 40-byte localhost)
//!   QUIC p50:  ~2 000 µs  (iggy/#606; picoquic loopback ~200–500 µs min)
//!   CMP p50:   ~10 µs     (cmp_rtt_bench; this repo)
//!   raw UDP:   ~2 µs      (udp_rtt_bench; this repo)
//!
//! QUIC adds ~2× TCP overhead on loopback because TLS AES-GCM
//! encrypts every record and the Tokio reactor wake-up cost is
//! identical. On a trusted LAN, both overheads are unnecessary
//! compared to CMP/UDP with no encryption and no TCP setup.
//!
//! See compare/quinn.md for protocol details and design comparison.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use quinn::ClientConfig;
use quinn::Endpoint;
use quinn::ServerConfig;
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::runtime::Builder;

// --- QUIC bench ---

fn bench_quinn_rtt(c: &mut Criterion) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    let (server_ep, client_ep, conn) = rt.block_on(async {
        let cert =
            rcgen::generate_simple_self_signed(vec!["localhost".into()])
                .unwrap();
        let cert_der = cert.cert.der().clone();
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

        let srv_cfg =
            ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())
                .unwrap();

        let server_ep = Endpoint::server(
            srv_cfg,
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        )
        .unwrap();
        let srv_addr = server_ep.local_addr().unwrap();

        // Echo server: accept one connection, echo every stream.
        let server_ep2 = server_ep.clone();
        tokio::spawn(async move {
            while let Some(incoming) = server_ep2.accept().await {
                let conn = incoming.await.unwrap();
                tokio::spawn(async move {
                    while let Ok((mut send, mut recv)) =
                        conn.accept_bi().await
                    {
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 256];
                            if let Ok(Some(n)) = recv.read(&mut buf).await {
                                let _ = send.write_all(&buf[..n]).await;
                            }
                        });
                    }
                });
            }
        });

        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert_der).unwrap();
        let cli_cfg =
            ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();

        let mut client_ep = Endpoint::client(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        )
        .unwrap();
        client_ep.set_default_client_config(cli_cfg);
        let conn = client_ep
            .connect(srv_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        (server_ep, client_ep, conn)
    });

    let payload = [0xAAu8; 64];
    let mut recv_buf = vec![0u8; 256];

    c.bench_function("quinn_rtt_loopback_64b", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (mut send, mut recv) = conn.open_bi().await.unwrap();
                send.write_all(black_box(&payload)).await.unwrap();
                send.finish().unwrap();
                let n = recv.read(&mut recv_buf).await.unwrap().unwrap_or(0);
                black_box(n);
            });
        });
    });

    rt.block_on(async {
        conn.close(0u32.into(), b"bench done");
        client_ep.wait_idle().await;
        server_ep.wait_idle().await;
    });
}

// --- TCP bench ---

fn bench_tcp_rtt(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    let (srv_addr, mut stream) = rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 256];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if sock.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        stream.set_nodelay(true).unwrap();
        (addr, stream)
    });
    let _ = srv_addr;

    let payload = [0xAAu8; 64];
    let mut recv_buf = vec![0u8; 256];

    c.bench_function("tcp_rtt_loopback_64b", |b| {
        b.iter(|| {
            rt.block_on(async {
                stream.write_all(black_box(&payload)).await.unwrap();
                let n = stream.read(&mut recv_buf).await.unwrap();
                black_box(n);
            });
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_quinn_rtt, bench_tcp_rtt
}
criterion_main!(benches);
