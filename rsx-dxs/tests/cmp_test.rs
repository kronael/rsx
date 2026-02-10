use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::header::WalHeader;
use rsx_dxs::records::PayloadPreamble;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::StatusMessage;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn loopback_pair() -> (CmpSender, CmpReceiver) {
    let wal_dir = PathBuf::from("./tmp/cmp_test_wal");
    let _ = std::fs::create_dir_all(&wal_dir);

    // Bind receiver first to get its address
    let tmp_recv = CmpReceiver::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:1".parse().unwrap(),
        1,
    )
    .unwrap();
    let recv_addr = tmp_recv.local_addr().unwrap();
    drop(tmp_recv);

    // Create sender targeting receiver
    let sender =
        CmpSender::new(recv_addr, 1, &wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();

    // Recreate receiver with correct sender_addr
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
        preamble: PayloadPreamble {
            seq,
            ver: 1,
            kind: 0,
            _pad0: 0,
            len: std::mem::size_of::<FillRecord>() as u32,
        },
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: 50000,
        qty: 100,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
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
    let (mut sender, mut receiver) = loopback_pair();
    let fill = fill_payload(1);
    sender
        .send_record(RECORD_FILL, as_bytes(&fill))
        .unwrap();

    thread::sleep(Duration::from_millis(10));

    let result = receiver.try_recv();
    assert!(result.is_some());
    let (preamble, payload) = result.unwrap();
    assert_eq!(preamble.record_type, RECORD_FILL);
    assert_eq!(
        payload.len(),
        std::mem::size_of::<FillRecord>(),
    );
    let decoded = unsafe {
        std::ptr::read(
            payload.as_ptr() as *const FillRecord,
        )
    };
    assert_eq!(decoded.preamble.seq, 1);
    assert_eq!(decoded.price, 50000);
    assert_eq!(decoded.qty, 100);
}

#[test]
fn sender_seq_increments() {
    let (mut sender, _receiver) = loopback_pair();
    assert_eq!(sender.next_seq(), 1);
    let fill = fill_payload(1);
    sender
        .send_record(RECORD_FILL, as_bytes(&fill))
        .unwrap();
    assert_eq!(sender.next_seq(), 2);
    let fill2 = fill_payload(2);
    sender
        .send_record(RECORD_FILL, as_bytes(&fill2))
        .unwrap();
    assert_eq!(sender.next_seq(), 3);
}

#[test]
fn status_message_updates_sender_window() {
    let (mut sender, _receiver) = loopback_pair();
    let msg = StatusMessage {
        stream_id: 1,
        _pad0: 0,
        consumption_seq: 42,
        receiver_window: 1024,
        _pad1: [0u8; 40],
    };
    sender.handle_status(&msg);
    assert_eq!(sender.peer_consumption_seq(), 42);
}

#[test]
fn flow_control_stalls_sender() {
    let (mut sender, _receiver) = loopback_pair();
    let msg = StatusMessage {
        stream_id: 1,
        _pad0: 0,
        consumption_seq: 0,
        receiver_window: 1,
        _pad1: [0u8; 40],
    };
    sender.handle_status(&msg);

    let fill = fill_payload(1);
    let sent = sender
        .send_record(RECORD_FILL, as_bytes(&fill))
        .unwrap();
    assert!(sent);

    let fill2 = fill_payload(2);
    let sent2 = sender
        .send_record(RECORD_FILL, as_bytes(&fill2))
        .unwrap();
    assert!(!sent2);
}

#[test]
fn receiver_expected_seq_advances() {
    let (mut sender, mut receiver) = loopback_pair();
    assert_eq!(receiver.expected_seq(), 1);

    let fill = fill_payload(1);
    sender
        .send_record(RECORD_FILL, as_bytes(&fill))
        .unwrap();
    thread::sleep(Duration::from_millis(10));
    receiver.try_recv();
    assert_eq!(receiver.expected_seq(), 2);
}

#[test]
fn multiple_records_in_order() {
    let (mut sender, mut receiver) = loopback_pair();
    for i in 1..=5u64 {
        let fill = fill_payload(i);
        sender
            .send_record(
                RECORD_FILL,
                as_bytes(&fill),
            )
            .unwrap();
    }
    thread::sleep(Duration::from_millis(20));

    let mut seqs = Vec::new();
    while let Some((_, payload)) = receiver.try_recv() {
        let decoded = unsafe {
            std::ptr::read(
                payload.as_ptr() as *const FillRecord,
            )
        };
        seqs.push(decoded.preamble.seq);
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
        payload.len() as u32,
        1,
        bad_crc,
    );
    let hdr_bytes = preamble.to_bytes();
    let mut buf = vec![0u8; 16 + payload.len()];
    buf[..16].copy_from_slice(&hdr_bytes);
    buf[16..].copy_from_slice(payload);
    sock.send_to(&buf, recv_local).unwrap();

    thread::sleep(Duration::from_millis(10));
    let result = receiver.try_recv();
    assert!(result.is_none());
}
