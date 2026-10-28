//! Risk's order stream to an ME is WAL-backed: an order lost on the UDP hop
//! is retransmitted from the WAL, each record carries the stream's own
//! contiguous seq, and a stream reopened by the next main continues the
//! numbering instead of restarting at 1.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::oldest_and_highest_seq;
use rsx_messages::CancelRequest;
use rsx_messages::OrderMessage;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_risk::rings::EgressStream;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const DEADLINE: Duration = Duration::from_secs(5);
const SYMBOL_ID: u32 = 3;

fn cancel(order: u64) -> CancelRequest {
    CancelRequest {
        seq: 0,
        ts_ns: 1_000_000_000 + order,
        user_id: 10,
        symbol_id: SYMBOL_ID,
        order_id_hi: 0,
        order_id_lo: order,
        _pad: [0; 24],
    }
}

fn order(order: u64) -> OrderMessage {
    OrderMessage {
        seq: 0,
        price: 100,
        qty: 5,
        side: 0,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        _pad1: [0; 4],
        user_id: 10,
        _pad2: 0,
        timestamp_ns: 1_000_000_000 + order,
        order_id_hi: 0,
        order_id_lo: order,
        risk_in_ns: 0,
        _pad3: [0; 56],
    }
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

fn open_stream(proxy: &DropProxy, wal_dir: &TempDir) -> EgressStream {
    EgressStream::open(
        SYMBOL_ID,
        proxy.addr,
        wal_dir.path(),
        &CastConfig::default(),
    )
    .unwrap()
}

fn receiver_for(proxy: &DropProxy, stream: &EgressStream) -> CastReceiver {
    let config = CastConfig {
        nak_debounce_us: 2_000,
        max_nak_retries: 3,
        ..Default::default()
    };
    let sender_addr = SocketAddr::from(([127, 0, 0, 1], stream.local_addr().unwrap().port()));
    let receiver =
        CastReceiver::with_config("127.0.0.1:0".parse().unwrap(), sender_addr, &config).unwrap();
    proxy.target.send(receiver.local_addr().unwrap()).unwrap();
    receiver
}

/// Tick `stream` and read `receiver` until `want` records arrived in total,
/// the receiver faults or the deadline passes; each delivered record's type
/// and seq is appended to `delivered`.
fn collect(
    receiver: &mut CastReceiver,
    stream: &mut EgressStream,
    want: usize,
    delivered: &mut Vec<(u16, u64)>,
) {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        stream.tick(Instant::now());
        let outcome = receiver.try_recv_with(|hdr, payload| {
            delivered.push((hdr.record_type, extract_seq(payload).unwrap()));
        });
        match outcome {
            CastRecvWith::Data if delivered.len() >= want => return,
            CastRecvWith::Data => {}
            CastRecvWith::Empty => std::thread::sleep(Duration::from_millis(1)),
            CastRecvWith::Faulted { .. } | CastRecvWith::Reconnect { .. } => {
                panic!("receiver faulted after {delivered:?}")
            }
        }
    }
    panic!("timed out with {delivered:?}");
}

// one order lost on the UDP hop (144 B, past the send ring) -> the receiver
// NAKs and the WAL serves it
#[test]
fn dropped_order_is_retransmitted_from_the_wal() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(2);
    let mut stream = open_stream(&proxy, &tmp);
    let mut receiver = receiver_for(&proxy, &stream);

    for id in 1..=3 {
        stream.forward(&mut order(id)).unwrap();
    }
    let mut delivered = Vec::new();
    collect(&mut receiver, &mut stream, 3, &mut delivered);

    assert_eq!(proxy.dropped.load(Ordering::Relaxed), 1);
    assert_eq!(
        delivered,
        vec![
            (RECORD_ORDER_REQUEST, 1),
            (RECORD_ORDER_REQUEST, 2),
            (RECORD_ORDER_REQUEST, 3),
        ]
    );
}

// orders and cancels share one contiguous seq stream per ME
#[test]
fn orders_and_cancels_share_the_streams_seq() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(u64::MAX);
    let mut stream = open_stream(&proxy, &tmp);
    let mut receiver = receiver_for(&proxy, &stream);

    stream.forward(&mut order(1)).unwrap();
    stream.forward(&mut cancel(1)).unwrap();
    stream.forward(&mut order(2)).unwrap();
    let mut delivered = Vec::new();
    collect(&mut receiver, &mut stream, 3, &mut delivered);

    assert_eq!(
        delivered,
        vec![
            (RECORD_ORDER_REQUEST, 1),
            (RECORD_CANCEL_REQUEST, 2),
            (RECORD_ORDER_REQUEST, 3),
        ]
    );
}

// a stream reopened over the previous main's WAL numbers new records after
// its tail
#[test]
fn reopened_stream_continues_after_the_wal_tail() {
    let tmp = TempDir::new().unwrap();
    let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = CastConfig::default();
    let mut first =
        EgressStream::open(SYMBOL_ID, sink.local_addr().unwrap(), tmp.path(), &config).unwrap();
    for id in 1..=3 {
        first.forward(&mut order(id)).unwrap();
    }
    first.flush().unwrap();
    drop(first);

    let mut second =
        EgressStream::open(SYMBOL_ID, sink.local_addr().unwrap(), tmp.path(), &config).unwrap();
    second.forward(&mut order(4)).unwrap();
    second.flush().unwrap();

    let range = oldest_and_highest_seq(SYMBOL_ID, tmp.path()).unwrap();
    assert_eq!(range, Some((1, 4)));
}

// a restarted main's first order reaches an ME that already took three
// records from the previous main: the receiver delivers it, not drops it as
// a duplicate
#[test]
fn restarted_mains_first_order_is_not_dropped_as_a_duplicate() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(u64::MAX);
    let mut first = open_stream(&proxy, &tmp);
    let mut receiver = receiver_for(&proxy, &first);
    for id in 1..=3 {
        first.forward(&mut order(id)).unwrap();
    }
    let mut delivered = Vec::new();
    collect(&mut receiver, &mut first, 3, &mut delivered);
    first.flush().unwrap();
    drop(first);

    let mut second = open_stream(&proxy, &tmp);
    second.forward(&mut order(4)).unwrap();
    collect(&mut receiver, &mut second, 4, &mut delivered);

    assert_eq!(
        delivered,
        vec![
            (RECORD_ORDER_REQUEST, 1),
            (RECORD_ORDER_REQUEST, 2),
            (RECORD_ORDER_REQUEST, 3),
            (RECORD_ORDER_REQUEST, 4),
        ]
    );
}
