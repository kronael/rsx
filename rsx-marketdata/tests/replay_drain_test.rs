//! `rsx_marketdata::replay::drain_replay` smoke test. Stands
//! up a real `ReplicationService` and drains; verifies apply
//! callback fires and `new_tip` matches.

use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_marketdata::replay::drain_replay;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::TempDir;

const STREAM_ID: u32 = 1;

fn reserve_port() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

/// Self-signed cert used as BOTH server identity and client CA
/// (single-box self-trust). Replication is TLS-mandatory.
fn test_tls(dir: &std::path::Path) -> rsx_cast::TlsConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("repl_cert.pem");
    let key_path = dir.join("repl_key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    rsx_cast::TlsConfig {
        server: Some(rsx_cast::TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: Some(rsx_cast::TlsClient { cert_path }),
    }
}

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: seq,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
        price: Price(100),
        qty: Qty(1),
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

#[test]
fn drain_replay_walks_wal_and_invokes_apply() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let mut writer = WalWriter::new(STREAM_ID, &wal_dir, 64 * 1024 * 1024).unwrap();
    for i in 1..=5u64 {
        let mut rec = fill(i);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
    let last_seq = writer.last_seq();
    assert_eq!(last_seq, 5);

    let tls = test_tls(tmp.path());
    let replay_addr = reserve_port();
    let wal_dir_srv = wal_dir.clone();
    let tls_srv = tls.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(wal_dir_srv, tls_srv).unwrap();
        rt.block_on(async move {
            service.serve(replay_addr).await.unwrap();
        });
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err() {
        if std::time::Instant::now() > deadline {
            panic!("replication server failed to bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let tip_file = tmp.path().join("md_replay_tip.bin");
    let mut applied_seqs: Vec<u64> = Vec::new();
    let new_tip = drain_replay(
        STREAM_ID,
        replay_addr.to_string(),
        0,
        tip_file,
        tls,
        |raw| {
            let seq = rsx_cast::wal::extract_seq(&raw.payload).unwrap_or(0);
            applied_seqs.push(seq);
        },
    )
    .expect("drain failed");

    assert_eq!(new_tip, last_seq);
    assert_eq!(applied_seqs, vec![1, 2, 3, 4, 5]);
}
