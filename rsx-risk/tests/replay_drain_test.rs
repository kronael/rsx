//! `rsx_risk::drain_replay` smoke test. Stands up a real
//! `ReplicationService` against a WAL, drains it, verifies
//! the apply callback was invoked for each record and the
//! returned `new_tip` matches the WAL tip.

use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_risk::drain_replay;
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

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000_000_000 + seq,
        symbol_id: 1,
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
        taker_ts_ns: 0,
    }
}

#[test]
fn drain_replay_walks_wal_and_invokes_apply() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let mut writer = WalWriter::new(
        STREAM_ID, &wal_dir, 64 * 1024 * 1024,
    )
    .unwrap();
    for i in 1..=4u64 {
        let mut rec = fill(i);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
    let last_seq = writer.last_seq();
    assert_eq!(last_seq, 4);

    let replay_addr = reserve_port();
    let wal_dir_srv = wal_dir.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(
            wal_dir_srv, None,
        )
        .unwrap();
        rt.block_on(async move {
            service.serve(replay_addr).await.unwrap();
        });
    });
    let deadline =
        std::time::Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err()
    {
        if std::time::Instant::now() > deadline {
            panic!("replication server failed to bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let tip_file = tmp.path().join("risk_replay_tip.bin");

    let mut applied_seqs: Vec<u64> = Vec::new();
    let new_tip = drain_replay(
        STREAM_ID,
        replay_addr.to_string(),
        0,
        tip_file,
        |raw| {
            let seq = rsx_cast::wal::extract_seq(&raw.payload)
                .unwrap_or(0);
            applied_seqs.push(seq);
        },
    )
    .expect("drain failed");

    assert_eq!(new_tip, last_seq);
    assert_eq!(applied_seqs, vec![1, 2, 3, 4]);
}

#[test]
fn drain_replay_skips_already_delivered() {
    // Same WAL, but last_delivered_seq=2 → apply only sees
    // seq=3,4.
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let mut writer = WalWriter::new(
        STREAM_ID, &wal_dir, 64 * 1024 * 1024,
    )
    .unwrap();
    for i in 1..=4u64 {
        let mut rec = fill(i);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();

    let replay_addr = reserve_port();
    let wal_dir_srv = wal_dir.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(
            wal_dir_srv, None,
        )
        .unwrap();
        rt.block_on(async move {
            service.serve(replay_addr).await.unwrap();
        });
    });
    let deadline =
        std::time::Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err()
    {
        if std::time::Instant::now() > deadline {
            panic!("replication server failed to bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let tip_file = tmp.path().join("risk_replay_tip2.bin");
    let mut applied_seqs: Vec<u64> = Vec::new();
    let new_tip = drain_replay(
        STREAM_ID,
        replay_addr.to_string(),
        2,
        tip_file,
        |raw| {
            let seq = rsx_cast::wal::extract_seq(&raw.payload)
                .unwrap_or(0);
            applied_seqs.push(seq);
        },
    )
    .expect("drain failed");

    assert_eq!(new_tip, 4);
    assert_eq!(applied_seqs, vec![3, 4]);
}
