use rsx_types::Price;
use rsx_types::Qty;
use rsx_dxs::config::TlsConfig;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::RECORD_CAUGHT_UP;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::DxsConsumer;
use rsx_dxs::DxsReplayService;
use rsx_dxs::WalWriter;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::TempDir;
use tokio::time::Duration;
use tokio::time::timeout;

fn generate_test_certs(
    dir: &std::path::Path,
) -> (PathBuf, PathBuf) {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");

    let cert = rcgen::generate_simple_self_signed(
        vec!["localhost".to_string()],
    )
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

    let (cert_path, key_path) =
        generate_test_certs(tmp.path());

    let server_tls_config = TlsConfig {
        enabled: true,
        cert_path: Some(cert_path.clone()),
        key_path: Some(key_path.clone()),
    };

    let client_tls_config = TlsConfig {
        enabled: true,
        cert_path: Some(cert_path.clone()),
        key_path: None,
    };

    let service = DxsReplayService::new(
        wal_dir.clone(),
        Some(server_tls_config),
    )
    .unwrap();

    // Use fixed test port to avoid bind-drop-rebind race
    let service_addr: SocketAddr =
        "127.0.0.1:19300".parse().unwrap();
    let service_clone = service.clone();
    let service_task = tokio::spawn(async move {
        service.serve(service_addr).await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let stream_id = 1u32;
    let mut wal = WalWriter::new(
        stream_id,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )
    .unwrap();

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
    };
    wal.append(&mut fill).unwrap();
    wal.flush().unwrap();

    let notify = service_clone.add_listener(stream_id).await;

    let tip_file = tmp.path().join("tip");
    let consumer_addr =
        format!("localhost:{}", service_addr.port());
    let mut consumer = DxsConsumer::new(
        stream_id,
        consumer_addr,
        tip_file,
        Some(client_tls_config),
    )
    .unwrap();

    let records = Arc::new(Mutex::new(Vec::new()));
    let records_clone = records.clone();

    let consumer_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(3),
            consumer.run(move |record| {
                let mut recs =
                    records_clone.lock().unwrap();
                recs.push(record);
            }),
        )
        .await;

        match result {
            Ok(_) => {}
            Err(_) => {}
        }
    });

    tokio::time::sleep(Duration::from_millis(200)).await;
    notify.notify_waiters();

    tokio::time::sleep(Duration::from_secs(2)).await;

    consumer_task.abort();
    service_task.abort();

    let recs = records.lock().unwrap();
    assert!(
        !recs.is_empty(),
        "expected at least one record via TLS, got {} records",
        recs.len()
    );

    let has_fill_or_caught_up = recs.iter().any(|r| {
        r.header.record_type == RECORD_FILL
            || r.header.record_type == RECORD_CAUGHT_UP
    });
    assert!(
        has_fill_or_caught_up,
        "expected FILL or CAUGHT_UP record"
    );
}

#[tokio::test]
async fn tls_disabled_falls_back_to_plain() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir(&wal_dir).unwrap();

    let service =
        DxsReplayService::new(wal_dir.clone(), None)
            .unwrap();

    // Use fixed test port to avoid bind-drop-rebind race
    let service_addr: SocketAddr =
        "127.0.0.1:19301".parse().unwrap();
    let service_clone = service.clone();
    let service_task = tokio::spawn(async move {
        service.serve(service_addr).await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let stream_id = 1u32;
    let mut wal = WalWriter::new(
        stream_id,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )
    .unwrap();

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
    };
    wal.append(&mut fill).unwrap();
    wal.flush().unwrap();

    let notify = service_clone.add_listener(stream_id).await;
    notify.notify_waiters();

    let tip_file = tmp.path().join("tip");
    let consumer_addr =
        format!("127.0.0.1:{}", service_addr.port());
    let mut consumer = DxsConsumer::new(
        stream_id,
        consumer_addr,
        tip_file,
        None,
    )
    .unwrap();

    let records = Arc::new(Mutex::new(Vec::new()));
    let records_clone = records.clone();

    let consumer_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(2),
            consumer.run(move |record| {
                let mut recs =
                    records_clone.lock().unwrap();
                recs.push(record);
                if recs.len() >= 1 {
                    return;
                }
            }),
        )
        .await;

        match result {
            Ok(_) => {}
            Err(_) => {}
        }
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    consumer_task.abort();
    service_task.abort();

    let recs = records.lock().unwrap();
    assert!(
        !recs.is_empty(),
        "expected at least one record via plain TCP"
    );

    let first = &recs[0];
    assert_eq!(first.header.record_type, RECORD_FILL);
}

#[test]
fn tls_config_validation_requires_cert_and_key() {
    let config = TlsConfig {
        enabled: true,
        cert_path: None,
        key_path: None,
    };

    let result = config.validate_server();
    assert!(result.is_err());
}

#[test]
fn tls_config_disabled_skips_validation() {
    let config = TlsConfig {
        enabled: false,
        cert_path: None,
        key_path: None,
    };

    let result = config.validate_server();
    assert!(result.is_ok());
}
