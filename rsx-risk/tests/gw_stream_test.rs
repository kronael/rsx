//! Risk's gateway egress stream is WAL-backed: a record lost on the UDP hop
//! is retransmitted from the WAL, a FAULTED gap is replayed from the
//! replication service over the same WAL, and a reopened stream continues
//! after the WAL tail.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::oldest_and_highest_seq;
use rsx_cast::ReplicationService;
use rsx_messages::FillRecord;
use rsx_messages::GW_STREAM_ID;
use rsx_messages::RECORD_FILL;
use rsx_risk::drain_replay;
use rsx_risk::rings::GwStream;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const DEADLINE: Duration = Duration::from_secs(5);

fn fill(order: u64) -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 1_000_000_000 + order,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
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
        _pad1: [0; 4],
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

fn serve_replication(wal_dir: &Path, tls: rsx_cast::TlsConfig) -> SocketAddr {
    let addr = reserve_port();
    let wal_dir = wal_dir.to_path_buf();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(wal_dir, tls).unwrap();
        runtime.block_on(async move {
            service.serve(addr).await.unwrap();
        });
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(addr).is_err() {
        assert!(
            Instant::now() < deadline,
            "replication server failed to bind"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    addr
}

/// A UDP hop between the stream and its receiver that drops the
/// `drop_nth` datagram it sees (1-based) and forwards the rest.
struct DropProxy {
    addr: SocketAddr,
    target: mpsc::Sender<SocketAddr>,
    dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl DropProxy {
    fn start(drop_nth: u64) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let addr = socket.local_addr().unwrap();
        let (target, target_rx) = mpsc::channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_dropped = dropped.clone();
        let thread_stop = stop.clone();
        std::thread::spawn(move || {
            let target: SocketAddr = target_rx.recv().unwrap();
            let mut buf = [0u8; 2048];
            let mut seen = 0u64;
            while !thread_stop.load(Ordering::Relaxed) {
                let Ok((len, _)) = socket.recv_from(&mut buf) else {
                    continue;
                };
                seen += 1;
                if seen == drop_nth {
                    thread_dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                socket.send_to(&buf[..len], target).unwrap();
            }
        });
        Self {
            addr,
            target,
            dropped,
            stop,
        }
    }
}

impl Drop for DropProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn loopback(stream: &GwStream) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], stream.local_addr().unwrap().port()))
}

fn receiver_config() -> CastConfig {
    CastConfig {
        nak_debounce_us: 2_000,
        max_nak_retries: 3,
        ..Default::default()
    }
}

/// Tick `stream` and read `receiver` until `want` records arrived, the
/// receiver faults or the deadline passes.
fn collect(
    receiver: &mut CastReceiver,
    stream: &mut GwStream,
    want: usize,
    seqs: &mut Vec<u64>,
) -> CastRecvWith {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        stream.tick(Instant::now());
        let outcome = receiver.try_recv_with(|hdr, payload| {
            assert_eq!(hdr.record_type, RECORD_FILL);
            seqs.push(extract_seq(payload).unwrap());
        });
        match outcome {
            CastRecvWith::Data if seqs.len() >= want => return outcome,
            CastRecvWith::Data => {}
            CastRecvWith::Empty => std::thread::sleep(Duration::from_millis(1)),
            CastRecvWith::Faulted { .. } | CastRecvWith::Reconnect { .. } => return outcome,
        }
    }
    panic!("timed out with {seqs:?}");
}

// one FILL lost on the UDP hop -> the receiver NAKs and the WAL serves it
#[test]
fn dropped_fill_is_retransmitted_from_the_wal() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(2);
    let mut stream = GwStream::open(proxy.addr, tmp.path(), &CastConfig::default()).unwrap();
    let mut receiver = CastReceiver::with_config(
        "127.0.0.1:0".parse().unwrap(),
        loopback(&stream),
        &receiver_config(),
    )
    .unwrap();
    proxy.target.send(receiver.local_addr().unwrap()).unwrap();

    for order in 1..=3 {
        stream.forward(&mut fill(order));
    }
    let mut seqs = Vec::new();
    collect(&mut receiver, &mut stream, 3, &mut seqs);

    assert_eq!(proxy.dropped.load(Ordering::Relaxed), 1);
    assert_eq!(seqs, vec![1, 2, 3]);
}

// a gap the NAK cannot close -> FAULTED; the replication service over the
// stream's WAL replays it and every record is delivered once
#[test]
fn faulted_gap_replays_from_the_embedded_service_once() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(2);
    let mut stream = GwStream::open(proxy.addr, tmp.path(), &CastConfig::default()).unwrap();
    let nak_sink = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut receiver = CastReceiver::with_config(
        "127.0.0.1:0".parse().unwrap(),
        nak_sink.local_addr().unwrap(),
        &receiver_config(),
    )
    .unwrap();
    proxy.target.send(receiver.local_addr().unwrap()).unwrap();
    let tls = test_tls(tmp.path());
    let replay_addr = serve_replication(tmp.path(), tls.clone());

    for order in 1..=5 {
        stream.forward(&mut fill(order));
    }
    stream.flush().unwrap();
    let mut seqs = Vec::new();
    let CastRecvWith::Faulted {
        last_delivered_seq, ..
    } = collect(&mut receiver, &mut stream, usize::MAX, &mut seqs)
    else {
        panic!("expected FAULTED");
    };
    assert_eq!(seqs, vec![1]);

    let new_tip = drain_replay(
        GW_STREAM_ID,
        replay_addr.to_string(),
        last_delivered_seq,
        tmp.path().join("gateway_replay_tip.bin"),
        tls,
        |raw| seqs.push(extract_seq(&raw.payload).unwrap()),
    )
    .unwrap();
    assert_eq!(new_tip, 5);
    receiver.reset_after_replay(new_tip);

    stream.forward(&mut fill(6));
    collect(&mut receiver, &mut stream, 6, &mut seqs);
    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);
}

// a stream opened over an existing WAL numbers new records after its tail
#[test]
fn reopened_stream_continues_after_the_wal_tail() {
    let tmp = TempDir::new().unwrap();
    let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut first = GwStream::open(
        sink.local_addr().unwrap(),
        tmp.path(),
        &CastConfig::default(),
    )
    .unwrap();
    for order in 1..=3 {
        first.forward(&mut fill(order));
    }
    first.flush().unwrap();
    drop(first);

    let mut second = GwStream::open(
        sink.local_addr().unwrap(),
        tmp.path(),
        &CastConfig::default(),
    )
    .unwrap();
    second.forward(&mut fill(4));
    second.flush().unwrap();

    let range = oldest_and_highest_seq(GW_STREAM_ID, tmp.path()).unwrap();
    assert_eq!(range, Some((1, 4)));
}
