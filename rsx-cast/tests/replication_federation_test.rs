//! Endpoint-list federation fallthrough (v0.4.0).
//!
//! `ReplicationConsumer::new` takes a Vec of endpoints. On a
//! `RECORD_REPLICATION_NOT_AVAILABLE` reply (the server can't
//! serve the requested `from_seq`) the consumer closes that
//! connection and retries the SAME `from_seq` against the next
//! endpoint. Every other consumer test uses a single-element
//! endpoint vec, so this path had zero coverage.
//!
//! Here endpoint A has an EMPTY WAL for the stream → it refuses
//! a from_seq=1 request with NOT_AVAILABLE. Endpoint B holds
//! the records → it serves them. The test asserts the consumer
//! falls through A and delivers B's records.

use rsx_cast::ReplicationConsumer;
use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_messages::RECORD_FILL;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::time::timeout;
use tokio::time::Duration;
use tempfile::TempDir;

const STREAM_ID: u32 = 1;

fn reserve_port() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000 + seq,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
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

/// Self-signed cert usable as BOTH server identity and client CA
/// (single-box self-trust). SAN covers localhost + 127.0.0.1 so a
/// consumer connecting to either verifies.
fn test_tls(dir: &std::path::Path) -> rsx_cast::TlsConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider()
        .install_default();
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ])
    .unwrap();
    let cert_path = dir.join("repl_cert.pem");
    let key_path = dir.join("repl_key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem())
        .unwrap();
    rsx_cast::TlsConfig {
        server: Some(rsx_cast::TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: Some(rsx_cast::TlsClient { cert_path }),
    }
}

async fn serve(
    wal_dir: std::path::PathBuf,
    addr: SocketAddr,
    tls: rsx_cast::TlsConfig,
) {
    let service =
        ReplicationService::new(wal_dir, tls).unwrap();
    service.serve(addr).await.unwrap();
}

async fn wait_bind(addr: SocketAddr) {
    let deadline =
        std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if std::net::TcpStream::connect(addr).is_ok() {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("replication server failed to bind {addr}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn not_available_falls_through_to_next_endpoint() {
    let tmp = TempDir::new().unwrap();

    // Endpoint A: empty WAL dir → refuses from_seq=1 with
    // NOT_AVAILABLE (oldest_and_highest_seq == None, and a
    // non-zero from_seq cannot be served).
    let dir_a = tmp.path().join("a");
    std::fs::create_dir_all(&dir_a).unwrap();

    // Endpoint B: WAL holding seq 1..=3 → serves the range.
    let dir_b = tmp.path().join("b");
    std::fs::create_dir_all(&dir_b).unwrap();
    let mut writer = WalWriter::new(
        STREAM_ID, &dir_b, 64 * 1024 * 1024,
    )
    .unwrap();
    for i in 1..=3u64 {
        let mut rec = fill(i);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);

    let tls = test_tls(tmp.path());
    let addr_a = reserve_port();
    let addr_b = reserve_port();
    tokio::spawn(serve(dir_a, addr_a, tls.clone()));
    tokio::spawn(serve(dir_b, addr_b, tls.clone()));
    wait_bind(addr_a).await;
    wait_bind(addr_b).await;

    // Consumer: A first (will NOT_AVAILABLE), then B. tip=0 →
    // requests from_seq=1.
    let tip_file = tmp.path().join("tip.bin");
    let mut consumer = ReplicationConsumer::new(
        STREAM_ID,
        vec![
            format!("127.0.0.1:{}", addr_a.port()),
            format!("127.0.0.1:{}", addr_b.port()),
        ],
        tip_file,
        tls,
    )
    .unwrap();

    let got: Arc<Mutex<Vec<u64>>> =
        Arc::new(Mutex::new(Vec::new()));
    let got_cb = got.clone();

    // run_once streams until the callback returns false; stop
    // after the three fills land so the test terminates.
    let _ = timeout(
        Duration::from_secs(5),
        consumer.run_once(move |raw| {
            if raw.header.record_type == RECORD_FILL {
                if let Some(seq) =
                    rsx_cast::wal::extract_seq(&raw.payload)
                {
                    got_cb.lock().unwrap().push(seq);
                }
            }
            got_cb.lock().unwrap().len() < 3
        }),
    )
    .await
    .expect("federation fallthrough timed out");

    let seqs = got.lock().unwrap().clone();
    assert_eq!(
        seqs,
        vec![1, 2, 3],
        "consumer must fall through endpoint A (NOT_AVAILABLE) \
         and receive endpoint B's records",
    );
}
