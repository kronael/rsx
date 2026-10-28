//! Risk casts to an ME through the sender's send ring: a cancel lost on the
//! UDP hop is retransmitted from the ring, and each record carries the
//! sender's own contiguous seq.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::extract_seq;
use rsx_messages::CancelRequest;
use rsx_messages::OrderMessage;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_risk::rings::send_to_me;
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

fn cancel(order: u64) -> CancelRequest {
    CancelRequest {
        seq: 0,
        ts_ns: 1_000_000_000 + order,
        user_id: 10,
        symbol_id: 3,
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

/// A UDP hop between the sender and its receiver that drops the
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

fn receiver_for(proxy: &DropProxy, sender: &CastSender) -> CastReceiver {
    let config = CastConfig {
        nak_debounce_us: 2_000,
        max_nak_retries: 3,
        ..Default::default()
    };
    let sender_addr = SocketAddr::from(([127, 0, 0, 1], sender.local_addr().unwrap().port()));
    let receiver =
        CastReceiver::with_config("127.0.0.1:0".parse().unwrap(), sender_addr, &config).unwrap();
    proxy.target.send(receiver.local_addr().unwrap()).unwrap();
    receiver
}

/// Tick `sender` and read `receiver` until `want` records arrived, the
/// receiver faults or the deadline passes; returns each delivered record's
/// type and seq.
fn collect(receiver: &mut CastReceiver, sender: &mut CastSender, want: usize) -> Vec<(u16, u64)> {
    let mut delivered = Vec::new();
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        sender.tick().unwrap();
        sender.recv_control();
        let outcome = receiver.try_recv_with(|hdr, payload| {
            delivered.push((hdr.record_type, extract_seq(payload).unwrap()));
        });
        match outcome {
            CastRecvWith::Data if delivered.len() >= want => return delivered,
            CastRecvWith::Data => {}
            CastRecvWith::Empty => std::thread::sleep(Duration::from_millis(1)),
            CastRecvWith::Faulted { .. } | CastRecvWith::Reconnect { .. } => {
                panic!("receiver faulted after {delivered:?}")
            }
        }
    }
    panic!("timed out with {delivered:?}");
}

// one cancel lost on the UDP hop -> the receiver NAKs and the send ring serves it
#[test]
fn dropped_cancel_is_retransmitted_from_the_ring() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(2);
    let mut sender =
        CastSender::with_config(proxy.addr, 3, tmp.path(), &CastConfig::default()).unwrap();
    let mut receiver = receiver_for(&proxy, &sender);

    for id in 1..=3 {
        send_to_me(&mut sender, &mut cancel(id)).unwrap();
    }
    let delivered = collect(&mut receiver, &mut sender, 3);

    assert_eq!(proxy.dropped.load(Ordering::Relaxed), 1);
    assert_eq!(
        delivered,
        vec![
            (RECORD_CANCEL_REQUEST, 1),
            (RECORD_CANCEL_REQUEST, 2),
            (RECORD_CANCEL_REQUEST, 3),
        ]
    );
}

// orders and cancels share one contiguous seq stream per ME
#[test]
fn orders_and_cancels_share_the_senders_seq() {
    let tmp = TempDir::new().unwrap();
    let proxy = DropProxy::start(u64::MAX);
    let mut sender =
        CastSender::with_config(proxy.addr, 3, tmp.path(), &CastConfig::default()).unwrap();
    let mut receiver = receiver_for(&proxy, &sender);

    send_to_me(&mut sender, &mut order(1)).unwrap();
    send_to_me(&mut sender, &mut cancel(1)).unwrap();
    send_to_me(&mut sender, &mut order(2)).unwrap();
    let delivered = collect(&mut receiver, &mut sender, 3);

    assert_eq!(
        delivered,
        vec![
            (RECORD_ORDER_REQUEST, 1),
            (RECORD_CANCEL_REQUEST, 2),
            (RECORD_ORDER_REQUEST, 3),
        ]
    );
}
