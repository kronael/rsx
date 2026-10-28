use crate::config::LiquidationConfig;
use crate::config::ReplicationConfig;
use crate::config::ShardConfig;
use crate::funding::FundingConfig;
use crate::promotion::classify_drain;
use crate::promotion::drain_round;
use crate::promotion::drain_until_caught_up;
use crate::promotion::Drain;
use crate::promotion::WarmStream;
use crate::promotion::FINAL_DRAIN_ROUNDS;
use crate::shard::RiskShard;
use rsx_cast::ReplicationConsumer;
use rsx_cast::ReplicationService;
use rsx_cast::TlsClient;
use rsx_cast::TlsConfig;
use rsx_cast::TlsServer;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::time::Duration;

#[test]
fn drain_returns_ok_once_the_tip_stops_advancing() {
    let rounds = [(Some(10), 5), (Some(12), 11), (Some(12), 12)];
    let mut next = 0;
    let result = drain_until_caught_up(Duration::ZERO, || {
        let (live_seq, applied_seq) = rounds[next];
        next += 1;
        Ok(classify_drain(live_seq, applied_seq))
    });
    assert!(result.is_ok());
    assert_eq!(next, 3);
}

#[test]
fn drain_errors_when_the_stream_ends_without_a_marker() {
    let mut calls = 0;
    let result = drain_until_caught_up(Duration::ZERO, || {
        calls += 1;
        Ok(classify_drain(None, 7))
    });
    assert!(result.is_err());
    assert_eq!(calls, 1);
}

#[test]
fn drain_gives_up_after_the_round_bound() {
    let mut calls = 0;
    let result = drain_until_caught_up(Duration::ZERO, || {
        calls += 1;
        Ok(Drain::Behind)
    });
    assert!(result.is_err());
    assert_eq!(calls, FINAL_DRAIN_ROUNDS);
}

#[test]
fn drain_propagates_a_stream_error() {
    let result = drain_until_caught_up(Duration::ZERO, || Err(std::io::Error::other("reset")));
    assert!(result.is_err());
}

const STREAM_ID: u32 = 1;

fn test_tls(dir: &std::path::Path) -> TlsConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("repl_cert.pem");
    let key_path = dir.join("repl_key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: Some(TlsClient { cert_path }),
    }
}

fn test_shard() -> RiskShard {
    RiskShard::new(ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: 2,
        symbol_params: vec![None; 2],
        taker_fee_bps: vec![0; 2],
        maker_fee_bps: vec![0; 2],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig {
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
        },
    })
}

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000_000_000 + seq,
        symbol_id: STREAM_ID,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
        price: Price(100),
        qty: Qty(5),
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

/// A `ReplicationService` over a WAL holding seq `1..=tail`, and a
/// warm stream whose persisted tip is `tip`.
fn serve_stream(
    dir: &std::path::Path,
    tail: u64,
    tip: u64,
) -> (tokio::runtime::Runtime, WarmStream) {
    let wal_dir = dir.join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();
    let mut writer = WalWriter::new(STREAM_ID, &wal_dir, 64 * 1024 * 1024).unwrap();
    for seq in 1..=tail {
        let mut record = fill(seq);
        let framed = writer.prepare(&mut record).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();

    let tls = test_tls(dir);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let replay_addr = listener.local_addr().unwrap();
    drop(listener);
    let service = ReplicationService::new(wal_dir, tls.clone()).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            service.serve(replay_addr).await.unwrap();
        });
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "replication server failed to bind"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut consumer = ReplicationConsumer::new(
        STREAM_ID,
        vec![format!("localhost:{}", replay_addr.port())],
        dir.join("tip.bin"),
        tls,
    )
    .unwrap();
    consumer.tip = tip;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let stream = WarmStream {
        symbol_id: STREAM_ID,
        consumer,
        applied_seq: tip,
    };
    (rt, stream)
}

#[test]
fn drain_is_caught_up_when_the_tip_equals_the_me_tail() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (rt, mut stream) = serve_stream(tmp.path(), 3, 3);
    let mut shard = test_shard();
    let result =
        drain_until_caught_up(Duration::ZERO, || drain_round(&rt, &mut stream, &mut shard));
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(stream.applied_seq, 3);
}

#[test]
fn drain_applies_the_records_past_the_tip_then_is_caught_up() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (rt, mut stream) = serve_stream(tmp.path(), 3, 1);
    let mut shard = test_shard();
    let result =
        drain_until_caught_up(Duration::ZERO, || drain_round(&rt, &mut stream, &mut shard));
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(stream.applied_seq, 3);
    assert_eq!(shard.fills_processed, 2);
}
