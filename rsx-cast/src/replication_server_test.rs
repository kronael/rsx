use crate::config::TlsClient;
use crate::config::TlsConfig;
use crate::config::TlsServer;
use crate::records::RECORD_CAUGHT_UP;
use crate::replication_client::ReplicationConsumer;
use crate::replication_server::ReplicationService;
use crate::wal::extract_seq;
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
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
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

fn corrupt_wal_fill() -> FillRecord {
    FillRecord {
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
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

/// A WAL with a corrupt record that intact records follow: the server streams
/// the records before it and drops the connection. It never sends
/// `CAUGHT_UP`, which would tell the consumer nothing is missing.
#[tokio::test]
async fn catch_up_over_a_corrupt_wal_never_reports_caught_up() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();
    let stream_id = 1u32;
    let mut wal = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();
    for _ in 0..3 {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
    }
    wal.flush().unwrap();
    drop(wal);
    let active = wal_dir.join("1").join("1_active.wal");
    let mut data = std::fs::read(&active).unwrap();
    let frame = data.len() / 3;
    data[frame + 16] ^= 0xFF;
    std::fs::write(&active, data).unwrap();

    let (cert_path, key_path) = generate_test_certs(tmp.path());
    let server_tls = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: None,
    };
    let client_tls = TlsConfig {
        server: None,
        client: Some(TlsClient { cert_path }),
    };
    let service = ReplicationService::new(wal_dir, server_tls).unwrap();
    let service_addr: SocketAddr = "127.0.0.1:19301".parse().unwrap();
    let service_task = tokio::spawn(async move { service.serve(service_addr).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![format!("localhost:{}", service_addr.port())],
        tmp.path().join("tip"),
        client_tls,
    )
    .unwrap();
    let record_types = Arc::new(Mutex::new(Vec::new()));
    let seen = record_types.clone();
    let _ = timeout(
        Duration::from_secs(2),
        consumer.run_once(move |record| {
            seen.lock().unwrap().push(record.header.record_type);
            true
        }),
    )
    .await;
    service_task.abort();

    assert_eq!(*record_types.lock().unwrap(), vec![RECORD_FILL]);
}

/// A consumer caught up on seqs 1..=10 then receives 11 and 12 once each: the
/// live tail sends only records past the last one it sent.
#[tokio::test]
async fn live_tail_sends_each_seq_once() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();
    let stream_id = 1u32;
    let mut wal = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();
    for _ in 0..10 {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
    }
    wal.flush().unwrap();

    let (cert_path, key_path) = generate_test_certs(tmp.path());
    let server_tls = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: None,
    };
    let client_tls = TlsConfig {
        server: None,
        client: Some(TlsClient { cert_path }),
    };
    let service = ReplicationService::new(wal_dir, server_tls).unwrap();
    let service_addr: SocketAddr = "127.0.0.1:19303".parse().unwrap();
    let service_task = tokio::spawn(async move { service.serve(service_addr).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![format!("localhost:{}", service_addr.port())],
        tmp.path().join("tip"),
        client_tls,
    )
    .unwrap();
    let seqs = Arc::new(Mutex::new(Vec::new()));
    let seen = seqs.clone();
    let consumer_task = tokio::spawn(async move {
        let _ = consumer
            .run_once(move |record| {
                if record.header.record_type == RECORD_FILL {
                    let seq = extract_seq(&record.payload).unwrap();
                    seen.lock().unwrap().push(seq);
                }
                true
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    for _ in 0..2 {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
    }
    wal.flush().unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    consumer_task.abort();
    service_task.abort();

    assert_eq!(*seqs.lock().unwrap(), (1..=12).collect::<Vec<u64>>());
}

/// After retention pruned the early segments and the producer restarted, a
/// consumer asking for seq 1 is refused, never served a short stream.
#[tokio::test]
async fn request_below_the_pruned_range_is_refused_after_restart() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();
    let stream_id = 1u32;

    let mut wal = WalWriter::new(stream_id, &wal_dir, 512).unwrap();
    for _ in 0..10 {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
        wal.flush().unwrap();
    }
    drop(wal);
    for entry in std::fs::read_dir(wal_dir.join("1")).unwrap() {
        let path = entry.unwrap().path();
        if !path.to_string_lossy().ends_with("_active.wal") {
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(
                std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 60 * 60),
            )
            .unwrap();
        }
    }
    let mut wal = WalWriter::new(stream_id, &wal_dir, 512).unwrap();
    wal.set_next_seq(11);
    for _ in 0..10 {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
        wal.flush().unwrap();
    }
    drop(wal);

    let (cert_path, key_path) = generate_test_certs(tmp.path());
    let server_tls = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: None,
    };
    let client_tls = TlsConfig {
        server: None,
        client: Some(TlsClient { cert_path }),
    };
    let service = ReplicationService::new(wal_dir, server_tls).unwrap();
    let service_addr: SocketAddr = "127.0.0.1:19302".parse().unwrap();
    let service_task = tokio::spawn(async move { service.serve(service_addr).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![format!("localhost:{}", service_addr.port())],
        tmp.path().join("tip"),
        client_tls,
    )
    .unwrap();
    let delivered = Arc::new(Mutex::new(0u32));
    let count = delivered.clone();
    let result = timeout(
        Duration::from_secs(2),
        consumer.run_once(move |_| {
            *count.lock().unwrap() += 1;
            true
        }),
    )
    .await
    .expect("run_once timed out");
    service_task.abort();

    let err = result.expect_err("seq 1 must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(*delivered.lock().unwrap(), 0);
}

/// A catch-up reply bigger than the socket buffer on a WAL that then stays
/// idle: `CAUGHT_UP` reaches the consumer without waiting for another record.
#[tokio::test]
async fn long_catch_up_reply_reaches_caught_up_on_an_idle_wal() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();
    let stream_id = 1u32;
    let record_count = 300usize;
    let mut wal = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();
    for _ in 0..record_count {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
    }
    wal.flush().unwrap();

    let (cert_path, key_path) = generate_test_certs(tmp.path());
    let server_tls = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: None,
    };
    let client_tls = TlsConfig {
        server: None,
        client: Some(TlsClient { cert_path }),
    };
    let service = ReplicationService::new(wal_dir, server_tls).unwrap();
    let service_addr: SocketAddr = "127.0.0.1:19304".parse().unwrap();
    let service_task = tokio::spawn(async move { service.serve(service_addr).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![format!("localhost:{}", service_addr.port())],
        tmp.path().join("tip"),
        client_tls,
    )
    .unwrap();
    let fills = Arc::new(Mutex::new(0usize));
    let seen = fills.clone();
    let result = timeout(
        Duration::from_secs(2),
        consumer.run_once(move |record| {
            if record.header.record_type == RECORD_FILL {
                *seen.lock().unwrap() += 1;
            }
            record.header.record_type != RECORD_CAUGHT_UP
        }),
    )
    .await;
    service_task.abort();

    let delivered = *fills.lock().unwrap();
    assert!(
        result.is_ok(),
        "no CAUGHT_UP within 2 s, {delivered} of {record_count} fills delivered"
    );
    assert_eq!(delivered, record_count);
}

/// A burst of live records bigger than the socket buffer, then an idle WAL:
/// every record reaches the consumer without waiting for another one.
#[tokio::test]
async fn long_live_burst_is_delivered_on_an_idle_wal() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();
    let stream_id = 1u32;
    let burst = 300usize;
    let mut wal = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();
    let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
    wal.append_framed(&framed).unwrap();
    wal.flush().unwrap();

    let (cert_path, key_path) = generate_test_certs(tmp.path());
    let server_tls = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: None,
    };
    let client_tls = TlsConfig {
        server: None,
        client: Some(TlsClient { cert_path }),
    };
    let service = ReplicationService::new(wal_dir, server_tls).unwrap();
    let service_addr: SocketAddr = "127.0.0.1:19305".parse().unwrap();
    let service_task = tokio::spawn(async move { service.serve(service_addr).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![format!("localhost:{}", service_addr.port())],
        tmp.path().join("tip"),
        client_tls,
    )
    .unwrap();
    let fills = Arc::new(Mutex::new(0usize));
    let seen = fills.clone();
    let consumer_task = tokio::spawn(async move {
        let _ = consumer
            .run_once(move |record| {
                if record.header.record_type == RECORD_FILL {
                    *seen.lock().unwrap() += 1;
                }
                true
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    for _ in 0..burst {
        let framed = wal.prepare(&mut corrupt_wal_fill()).unwrap();
        wal.append_framed(&framed).unwrap();
    }
    wal.flush().unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    consumer_task.abort();
    service_task.abort();

    assert_eq!(*fills.lock().unwrap(), burst + 1);
}
