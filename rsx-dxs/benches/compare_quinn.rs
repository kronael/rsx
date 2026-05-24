//! Quinn (QUIC) and TCP loopback — standalone benches.
//!
//! For a uniform multi-protocol harness see compare_all.rs.
//! This file runs QUIC and TCP in isolation for focused profiling.
//!
//! `quinn_rtt_new_stream`    — new stream per iteration (naive)
//! `quinn_rtt_persistent`   — persistent stream + framing (optimal)
//! `tcp_rtt_nodelay`        — TCP nodelay persistent connection

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

async fn read_framed(recv: &mut RecvStream, buf: &mut [u8]) -> usize {
    let mut len_buf = [0u8; 4];
    if recv.read_exact(&mut len_buf).await.is_err() { return 0; }
    let n = u32::from_le_bytes(len_buf) as usize;
    if recv.read_exact(&mut buf[..n]).await.is_err() { return 0; }
    n
}

async fn write_framed(send: &mut SendStream, data: &[u8]) {
    let len = (data.len() as u32).to_le_bytes();
    let _ = send.write_all(&len).await;
    let _ = send.write_all(data).await;
}

fn make_connected_pair(
    rt: &tokio::runtime::Runtime,
) -> (Endpoint, Endpoint, quinn::Connection, quinn::Connection) {
    rt.block_on(async {
        let cert =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_der = cert.cert.der().clone();
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

        let srv_cfg = ServerConfig::with_single_cert(
            vec![cert_der.clone()], key_der.into(),
        ).unwrap();
        let server_ep = Endpoint::server(
            srv_cfg, "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        ).unwrap();
        let srv_addr = server_ep.local_addr().unwrap();

        let server_ep2 = server_ep.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Some(inc) = server_ep2.accept().await {
                if let Ok(conn) = inc.await { let _ = tx.send(conn); }
            }
        });

        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert_der).unwrap();
        let cli_cfg = ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
        let mut client_ep = Endpoint::client(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        ).unwrap();
        client_ep.set_default_client_config(cli_cfg);
        let cli_conn = client_ep.connect(srv_addr, "localhost").unwrap().await.unwrap();
        let srv_conn = rx.await.expect("server accept");

        (server_ep, client_ep, cli_conn, srv_conn)
    })
}

// ── QUIC: new stream per iteration ───────────────────────────────────────────

fn bench_quinn_new_stream(c: &mut Criterion) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let (server_ep, client_ep, cli_conn, srv_conn) = make_connected_pair(&rt);

    rt.spawn(async move {
        while let Ok((mut send, mut recv)) = srv_conn.accept_bi().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 128];
                if let Ok(Some(n)) = recv.read(&mut buf).await {
                    let _ = send.write_all(&buf[..n]).await;
                }
            });
        }
    });

    let payload = [0xAAu8; 64];
    let mut recv_buf = [0u8; 128];

    c.bench_function("quinn_rtt_new_stream_64b", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (mut send, mut recv) = cli_conn.open_bi().await.unwrap();
                send.write_all(black_box(&payload)).await.unwrap();
                send.finish().unwrap();
                let n = recv.read(&mut recv_buf).await.unwrap().unwrap_or(0);
                black_box(n);
            });
        });
    });

    rt.block_on(async {
        cli_conn.close(0u32.into(), b"done");
        client_ep.wait_idle().await;
        server_ep.wait_idle().await;
    });
}

// ── QUIC: persistent stream + framing ────────────────────────────────────────

fn bench_quinn_persistent(c: &mut Criterion) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let (server_ep, client_ep, cli_conn, srv_conn) = make_connected_pair(&rt);

    let (mut cli_send, mut cli_recv, srv_send, srv_recv) = rt.block_on(async {
        let (cli_s, cli_r) = cli_conn.open_bi().await.unwrap();
        let (srv_s, srv_r) = srv_conn.accept_bi().await.unwrap();
        (cli_s, cli_r, srv_s, srv_r)
    });

    rt.spawn(async move {
        let mut s = srv_send;
        let mut r = srv_recv;
        let mut buf = [0u8; 256];
        loop {
            let n = read_framed(&mut r, &mut buf).await;
            if n == 0 { break; }
            write_framed(&mut s, &buf[..n]).await;
        }
    });

    let payload = [0xAAu8; 64];
    let mut recv_buf = [0u8; 256];

    c.bench_function("quinn_rtt_persistent_64b", |b| {
        b.iter(|| {
            rt.block_on(async {
                write_framed(&mut cli_send, black_box(&payload)).await;
                let n = read_framed(&mut cli_recv, &mut recv_buf).await;
                black_box(n);
            });
        });
    });

    rt.block_on(async {
        cli_conn.close(0u32.into(), b"done");
        client_ep.wait_idle().await;
        server_ep.wait_idle().await;
    });
}

// ── TCP nodelay ───────────────────────────────────────────────────────────────

fn bench_tcp_rtt(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    let mut stream = rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                sock.set_nodelay(true).unwrap();
                tokio::spawn(async move {
                    let mut buf = [0u8; 256];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => { if sock.write_all(&buf[..n]).await.is_err() { break; } }
                        }
                    }
                });
            }
        });
        let s = TcpStream::connect(addr).await.unwrap();
        s.set_nodelay(true).unwrap();
        s
    });

    let payload = [0xAAu8; 64];
    let mut recv_buf = [0u8; 256];

    c.bench_function("tcp_rtt_nodelay_64b", |b| {
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
    targets = bench_quinn_new_stream, bench_quinn_persistent, bench_tcp_rtt
}
criterion_main!(benches);
