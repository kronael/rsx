use crate::config::TlsClient;
use crate::config::TlsConfig;
use crate::config::TlsServer;
use crate::records::RECORD_CAUGHT_UP;
use crate::replication_client::ReplicationConsumer;
use crate::replication_server::ReplicationService;
use crate::wal::WalWriter;
use rsx_messages::FillRecord;
use rsx_messages::RECORD_FILL;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::TempDir;
use tokio::time::timeout;
use tokio::time::Duration;

fn generate_test_certs(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");

    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();

    std::fs::write(&cert_path, cert_pem).unwrap();
    std::fs::write(&key_path, key_pem).unwrap();

    (cert_path, key_path)
}

#[tokio::test]
async fn tls_client_server_connection() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();

    let (cert_path, key_path) = generate_test_certs(tmp.path());

    let server_tls = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
        }),
        client: None,
    };

    let client_tls = TlsConfig {
        server: None,
        client: Some(TlsClient {
            cert_path: cert_path.clone(),
        }),
    };

    let service = ReplicationService::new(wal_dir.clone(), server_tls).unwrap();

    let service_addr: SocketAddr = "127.0.0.1:19300".parse().unwrap();
    let service_task = tokio::spawn(async move { service.serve(service_addr).await });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let stream_id = 1u32;
    let mut wal = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();

    let mut fill = FillRecord {
        seq: 0,
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 100,
        maker_user_id: 200,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 1,
        maker_order_id_hi: 0,
        maker_order_id_lo: 2,
        price: Price(50000),
        qty: Qty(1000),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    };
    {
        let framed = wal.prepare(&mut fill).unwrap();
        wal.append_framed(&framed).unwrap();
    }
    wal.flush().unwrap();

    let tip_file = tmp.path().join("tip");
    let consumer_addr = format!("localhost:{}", service_addr.port());
    let mut consumer =
        ReplicationConsumer::new(stream_id, vec![consumer_addr], tip_file, client_tls).unwrap();

    let records = Arc::new(Mutex::new(Vec::new()));
    let records_clone = records.clone();

    let consumer_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(3),
            consumer.run(move |record| {
                let mut recs = records_clone.lock().unwrap();
                recs.push(record);
                true
            }),
        )
        .await;

        if result.is_ok() {}
    });

    tokio::time::sleep(Duration::from_secs(2)).await;

    consumer_task.abort();
    service_task.abort();

    let recs = records.lock().unwrap();
    assert!(
        !recs.is_empty(),
        "expected at least one record via TLS, got {} records",
        recs.len()
    );

    let has_fill_or_caught_up = recs
        .iter()
        .any(|r| r.header.record_type == RECORD_FILL || r.header.record_type == RECORD_CAUGHT_UP);
    assert!(has_fill_or_caught_up, "expected FILL or CAUGHT_UP record");
}

/// Replication is TLS-mandatory: a server config without the
/// `.server` (cert+key) half is rejected at construction.
#[test]
fn service_new_requires_server_cert() {
    let tmp = TempDir::new().unwrap();
    let client_only = TlsConfig {
        server: None,
        client: Some(TlsClient {
            cert_path: tmp.path().join("ca.pem"),
        }),
    };
    let err = ReplicationService::new(tmp.path().to_path_buf(), client_only)
        .err()
        .expect("expected TLS-mandatory construction error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

/// A consumer config without the `.client` (CA) half is
/// rejected at construction — no plaintext fallback.
#[test]
fn consumer_new_requires_client_ca() {
    let tmp = TempDir::new().unwrap();
    let server_only = TlsConfig {
        server: Some(TlsServer {
            cert_path: tmp.path().join("cert.pem"),
            key_path: tmp.path().join("key.pem"),
        }),
        client: None,
    };
    let err = ReplicationConsumer::new(
        1,
        vec!["127.0.0.1:1".to_string()],
        tmp.path().join("tip"),
        server_only,
    )
    .err()
    .expect("expected TLS-mandatory construction error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}
