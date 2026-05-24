use rsx_types::Price;
use rsx_types::Qty;
use rsx_cast::cmp::CmpRecv;
use rsx_cast::cmp::CmpReceiver;
use rsx_cast::cmp::CmpSender;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::protocol::Nak;
use rsx_messages::FillRecord;
use rsx_cast::protocol::RECORD_NAK;
use rsx_messages::RECORD_FILL;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn loopback_pair(wal_dir: &std::path::Path) -> (CmpSender, CmpReceiver) {
    // Use ephemeral port (0) for parallel test execution
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock); // Release immediately for CmpReceiver to bind

    // Create sender first (it will bind to an OS-assigned port)
    let sender = CmpSender::new(recv_addr, 1, wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();

    // Create receiver with sender's actual address
    let receiver = CmpReceiver::new(
        recv_addr,
        sender_addr,
        1,
    )
    .unwrap();

    (sender, receiver)
}

fn fill_payload(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: Price(50000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
taker_ts_ns: 0,
    }
}

fn as_bytes<T>(val: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            val as *const T as *const u8,
            std::mem::size_of::<T>(),
        )
    }
}

#[test]
fn send_recv_roundtrip() {
    let _tmp = TempDir::new().unwrap();
    let (mut sender, mut receiver) = loopback_pair(_tmp.path());
    let mut fill = fill_payload(1);
    sender.send(&mut fill).unwrap();

    thread::sleep(Duration::from_millis(10));

    let result = receiver.try_recv();
    let (preamble, payload) = match result {
        CmpRecv::Data(h, p) => (h, p),
        other => panic!("expected Data, got {other:?}"),
    };
    assert_eq!(preamble.record_type, RECORD_FILL);
    assert_eq!(
        payload.len(),
        std::mem::size_of::<FillRecord>(),
    );
    let decoded = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr() as *const FillRecord,
        )
    };
    assert_eq!(decoded.seq, 1);
    assert_eq!(decoded.price, Price(50000));
    assert_eq!(decoded.qty, Qty(100));
}

#[test]
fn sender_seq_increments() {
    let _tmp = TempDir::new().unwrap();
    let (mut sender, _receiver) = loopback_pair(_tmp.path());
    assert_eq!(sender.next_seq(), 1);
    let mut fill = fill_payload(1);
    sender.send(&mut fill).unwrap();
    assert_eq!(sender.next_seq(), 2);
    let mut fill2 = fill_payload(2);
    sender.send(&mut fill2).unwrap();
    assert_eq!(sender.next_seq(), 3);
}

// status_message_updates_sender_window + flow_control_stalls_sender
// removed: StatusMessage/flow-control deleted (exchange-grade
// transports don't have backpressure; consumers recover via DXS
// replay, not by stalling the producer).

#[test]
fn receiver_expected_seq_advances() {
    let _tmp = TempDir::new().unwrap();
    let (mut sender, mut receiver) = loopback_pair(_tmp.path());
    // Starts at 0 (unsynced); first packet syncs it
    assert_eq!(receiver.expected_seq(), 0);

    let mut fill = fill_payload(1);
    sender.send(&mut fill).unwrap();
    thread::sleep(Duration::from_millis(10));
    receiver.try_recv();
    assert_eq!(receiver.expected_seq(), 2);
}

#[test]
fn multiple_records_in_order() {
    let _tmp = TempDir::new().unwrap();
    let (mut sender, mut receiver) = loopback_pair(_tmp.path());
    for i in 1..=5u64 {
        let mut fill = fill_payload(i);
        sender.send(&mut fill).unwrap();
    }
    thread::sleep(Duration::from_millis(20));

    let mut seqs = Vec::new();
    while let CmpRecv::Data(_, payload) =
        receiver.try_recv()
    {
        let decoded = unsafe {
            std::ptr::read_unaligned(
                payload.as_ptr() as *const FillRecord,
            )
        };
        seqs.push(decoded.seq);
    }
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
}

#[test]
fn crc_mismatch_rejected() {
    let recv_addr: SocketAddr =
        "127.0.0.1:0".parse().unwrap();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let sender_addr = sock.local_addr().unwrap();

    let tmp = CmpReceiver::new(
        recv_addr,
        sender_addr,
        1,
    )
    .unwrap();
    let recv_local = tmp.local_addr().unwrap();
    drop(tmp);

    let mut receiver = CmpReceiver::new(
        recv_local,
        sender_addr,
        1,
    )
    .unwrap();

    let fill = fill_payload(1);
    let payload = as_bytes(&fill);
    let bad_crc = 0xDEADBEEFu32;
    let preamble = WalHeader::new(
        RECORD_FILL,
        payload.len() as u16,
        bad_crc,
    );
    let hdr_bytes = preamble.to_bytes();
    let mut buf = vec![0u8; 16 + payload.len()];
    buf[..16].copy_from_slice(&hdr_bytes);
    buf[16..].copy_from_slice(payload);
    sock.send_to(&buf, recv_local).unwrap();

    thread::sleep(Duration::from_millis(10));
    let result = receiver.try_recv();
    assert!(matches!(result, CmpRecv::Empty));
}

#[test]
fn nak_retransmit_from_wal() {
    let tmp = TempDir::new().unwrap();
    let (mut sender, mut receiver) =
        loopback_pair(tmp.path());
    let sender_addr = sender.local_addr().unwrap();

    // Send seq=1 so it lands in the send ring;
    // do not drain receiver so expected_seq stays 1
    let mut fill = fill_payload(0);
    sender.send(&mut fill).unwrap();

    // Send NAK for seq=1 to trigger ring retransmit
    let nak = Nak { from_seq: 1, count: 1, _pad1: [0u8; 48] };
    let payload = as_bytes(&nak);
    let crc = compute_crc32(payload);
    let hdr = WalHeader::new(
        RECORD_NAK,
        payload.len() as u16,
        crc,
    );
    let hdr_bytes = hdr.to_bytes();
    let mut buf = vec![0u8; 16 + payload.len()];
    buf[..16].copy_from_slice(&hdr_bytes);
    buf[16..].copy_from_slice(payload);
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.send_to(&buf, sender_addr).unwrap();

    // Process control and expect retransmit at receiver
    sender.recv_control();
    thread::sleep(Duration::from_millis(10));
    let result = receiver.try_recv();
    let (hdr, payload) = match result {
        CmpRecv::Data(h, p) => (h, p),
        other => panic!("expected Data, got {other:?}"),
    };
    assert_eq!(hdr.record_type, RECORD_FILL);
    let decoded = unsafe {
        std::ptr::read_unaligned(
            payload.as_ptr() as *const FillRecord,
        )
    };
    assert_eq!(decoded.seq, 1);
}

// cmp_sender_window_exhausted_blocks removed: flow-control window
// no longer exists. send() never stalls.

#[test]
fn cmp_heartbeat_sent_on_idle() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path();

    let tmp_recv = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = tmp_recv.local_addr().unwrap();
    tmp_recv.set_nonblocking(true).unwrap();

    let config = rsx_cast::config::CmpConfig {
        heartbeat_interval_ms: 5,
        ..Default::default()
    };
    let mut sender = CmpSender::with_config(
        recv_addr, 1, &wal_dir, &config,
    )
    .unwrap();

    thread::sleep(Duration::from_millis(10));
    sender.tick().unwrap();

    let mut buf = [0u8; 256];
    let result = tmp_recv.recv_from(&mut buf);
    assert!(
        result.is_ok(),
        "should have received heartbeat"
    );
    let (n, _) = result.unwrap();
    assert!(n >= WalHeader::SIZE);
    let hdr = WalHeader::from_bytes(&buf[..16]).unwrap();
    assert_eq!(hdr.record_type, rsx_cast::protocol::RECORD_HEARTBEAT);
}

