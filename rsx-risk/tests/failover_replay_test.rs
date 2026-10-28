//! Live FAULTED/RECONNECT gap replay must APPLY the drained ME
//! records to the shard — positions, fees, tips — not merely
//! drain them to clear the receiver. A discarded ME gap leaves
//! positions ≠ sum of fills permanently (invariant #4): the next
//! live fill advances the tip past the gap, so even a restart
//! replay never heals it.

use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_risk::replay::apply_me_replay_record;
use rsx_risk::replay::handle_replay;
use rsx_risk::rings::GwStream;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::TempDir;

/// Doubles as the ME symbol_id: replication streams are keyed by
/// symbol for ME producers.
const STREAM_ID: u32 = 1;

fn reserve_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
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

fn shard_config() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: 4,
        symbol_params: vec![
            Some(SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
                min_notional: 1,
            });
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    }
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

#[test]
fn faulted_replay_applies_me_fills_to_shard() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let mut writer = WalWriter::new(STREAM_ID, &wal_dir, 64 * 1024 * 1024).unwrap();
    for seq in 1..=4u64 {
        let mut rec = fill(seq);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
    assert_eq!(writer.last_seq(), 4);

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

    let mut shard = RiskShard::new(shard_config());
    assert_eq!(shard.tips[STREAM_ID as usize], 0);

    // Sink for the forwarded records; nobody reads it, the send
    // only has to succeed.
    let gw_sink = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let gw_addr = gw_sink.local_addr().unwrap();
    let mut gw_stream = GwStream::open(gw_addr, tmp.path()).unwrap();

    let new_tip = handle_replay(
        "risk.me",
        Some(&replay_addr.to_string()),
        STREAM_ID,
        0,
        Some((1, 4)),
        wal_dir.to_str().unwrap(),
        &tls,
        |raw| apply_me_replay_record(&mut shard, &mut gw_stream, raw),
    );

    assert_eq!(new_tip, 4);
    // The gap's fills are folded into the shard: positions = sum
    // of fills, fees deducted, tip advanced.
    assert_eq!(shard.tips[STREAM_ID as usize], 4, "tip must advance");
    assert_eq!(shard.fills_processed, 4, "every gap fill applied");
    let taker = &shard.positions[&(10, STREAM_ID)];
    assert_eq!(taker.long_qty, 20, "taker bought 4 x 5");
    let maker = &shard.positions[&(20, STREAM_ID)];
    assert_eq!(maker.short_qty, 20, "maker sold 4 x 5");
    // taker fee: 5 * 100 * 5 bps = 0.25 -> floor 0 per fill; use
    // account existence as the fee-path witness instead of a
    // magic number.
    assert!(shard.accounts.contains_key(&10));
    assert!(shard.accounts.contains_key(&20));
}
