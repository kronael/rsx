//! The gateway replication listener belongs to the main: it holds the shard's
//! address only between `start` and `stop`, serves the WAL directory to a
//! real consumer, and hands the address to the next `start`.

use rsx_cast::wal::extract_seq;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_messages::GW_STREAM_ID;
use rsx_risk::drain_replay;
use rsx_risk::gw_replication::GwReplication;
use rsx_types::Price;
use rsx_types::Qty;
use std::io;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(5);

fn fill(order: u64) -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 1_000_000_000 + order,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        taker_fee_bps: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: order,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + order,
        price: Price(100),
        qty: Qty(5),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        maker_fee_bps: 0,
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

fn reserve_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn test_tls(dir: &Path) -> rsx_cast::TlsConfig {
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

fn write_fills(wal_dir: &Path, count: u64) {
    let mut writer = WalWriter::new(GW_STREAM_ID, wal_dir, 64 * 1024 * 1024).unwrap();
    for order in 1..=count {
        let mut record = fill(order);
        let framed = writer.prepare(&mut record).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
}

fn wait_until_accepting(addr: SocketAddr) {
    let deadline = Instant::now() + WAIT;
    while TcpStream::connect(addr).is_err() {
        assert!(
            Instant::now() < deadline,
            "listener never accepted on {addr}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn drain(addr: SocketAddr, tls: &rsx_cast::TlsConfig, tip_file: &Path) -> io::Result<Vec<u64>> {
    let mut seqs = Vec::new();
    drain_replay(
        GW_STREAM_ID,
        addr.to_string(),
        0,
        tip_file.to_path_buf(),
        tls.clone(),
        |raw| seqs.push(extract_seq(&raw.payload).unwrap()),
    )?;
    Ok(seqs)
}

fn start(
    wal_dir: &Path,
    tls: &rsx_cast::TlsConfig,
    addr: SocketAddr,
    wait: Duration,
) -> GwReplication {
    GwReplication::start(
        wal_dir.to_path_buf(),
        tls.clone(),
        addr,
        Instant::now() + wait,
    )
    .unwrap()
}

// while started the address is held and a consumer drains the WAL
#[test]
fn started_listener_holds_the_address_and_serves_the_wal() {
    let tmp = TempDir::new().unwrap();
    write_fills(tmp.path(), 3);
    let tls = test_tls(tmp.path());
    let addr = reserve_port();
    let replication = start(tmp.path(), &tls, addr, WAIT);
    wait_until_accepting(addr);

    let error = TcpListener::bind(addr).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    let seqs = drain(addr, &tls, &tmp.path().join("tip.bin")).unwrap();
    assert_eq!(seqs, vec![1, 2, 3]);

    replication.stop();
}

// after stop the address binds again, client connections notwithstanding
#[test]
fn stopped_listener_releases_the_address() {
    let tmp = TempDir::new().unwrap();
    write_fills(tmp.path(), 2);
    let tls = test_tls(tmp.path());
    let addr = reserve_port();
    let replication = start(tmp.path(), &tls, addr, WAIT);
    wait_until_accepting(addr);
    let seqs = drain(addr, &tls, &tmp.path().join("tip.bin")).unwrap();
    assert_eq!(seqs, vec![1, 2]);

    let stopped = Instant::now();
    replication.stop();
    let _listener = loop {
        match TcpListener::bind(addr) {
            Ok(listener) => break listener,
            Err(error) => {
                assert!(
                    stopped.elapsed() < Duration::from_secs(1),
                    "address still busy after stop: {error}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
}

// a second start on a held address waits, then serves once the first stops
#[test]
fn second_start_serves_after_the_first_stops() {
    let tmp = TempDir::new().unwrap();
    write_fills(tmp.path(), 4);
    let tls = test_tls(tmp.path());
    let addr = reserve_port();
    let first = start(tmp.path(), &tls, addr, WAIT);
    wait_until_accepting(addr);
    let second = start(tmp.path(), &tls, addr, WAIT);
    std::thread::sleep(Duration::from_millis(300));

    first.stop();
    let deadline = Instant::now() + WAIT;
    let seqs = loop {
        let drained = drain(addr, &tls, &tmp.path().join("tip.bin"));
        if let Ok(seqs) = drained {
            break seqs;
        }
        assert!(Instant::now() < deadline, "second listener never served");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(seqs, vec![1, 2, 3, 4]);

    second.stop();
}
