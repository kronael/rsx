//! WAL random-access NAK fallback — correctness.
//!
//! Send enough records to push past the send-ring horizon
//! (SEND_RING_CAPACITY = 4096), then NAK an early seq the ring no
//! longer holds. The retransmit must come from disk via
//! `read_record_at_seq`. This asserts the record round-trips; the
//! fallback's *latency* is owned by `wal_random_read_bench`, not a
//! wall-clock assertion here.

use rsx_cast::as_bytes;
use rsx_cast::compute_crc32;
use rsx_cast::CastReceiver;
use rsx_cast::CastRecv;
use rsx_cast::CastSender;
use rsx_cast::Framed;
use rsx_cast::Nak;
use rsx_cast::WalHeader;
use rsx_cast::WalWriter;
use rsx_cast::RECORD_NAK;
use rsx_messages::FillRecord;
use rsx_messages::RECORD_FILL;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::UdpSocket;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000 + seq,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: Price(50_000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    }
}

#[test]
fn nak_wal_fallback_delivers_evicted_seq() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path();

    // Populate the WAL with records 1..=5000 so they survive on
    // disk after the in-memory send ring has wrapped past them.
    // SEND_RING_CAPACITY is 4096.
    let stream_id = 1u32;
    let mut writer = WalWriter::new(stream_id, wal_dir, 64 * 1024 * 1024).unwrap();
    for i in 1..=5_000u64 {
        let mut rec = fill(i);
        {
            let framed = writer.prepare(&mut rec).unwrap();
            writer.append_framed(&framed).unwrap();
        }
    }
    writer.flush().unwrap();

    // Build sender/receiver loopback.
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock);
    let mut sender = CastSender::new(recv_addr, stream_id, wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver = CastReceiver::new(recv_addr, sender_addr).unwrap();

    // Push 5000 sends through the in-memory ring so the sender's
    // slot for seq=1 (slot = seq & 4095) has been overwritten.
    // Drain the receiver each batch so its reorder buffer doesn't
    // fill.
    let mut seq = 0u64;
    for _ in 0..5_000u64 {
        let mut rec = fill(0);
        seq += 1;
        sender.send_framed(&Framed::pack(&mut rec, seq)).unwrap();
        while matches!(receiver.try_recv(), CastRecv::Data(_, _)) {}
    }
    thread::sleep(Duration::from_millis(20));
    while matches!(receiver.try_recv(), CastRecv::Data(_, _)) {}

    // NAK seq=1 from a third-party socket — its ring slot has been
    // overwritten by seq=4097, so the sender must fall through to
    // the WAL.
    let target_seq = 1u64;
    let nak = Nak {
        from_seq: target_seq,
        count: 1,
        _pad1: [0u8; 48],
    };
    let payload = as_bytes(&nak);
    let crc = compute_crc32(payload);
    let hdr = WalHeader::new(RECORD_NAK, payload.len() as u16, crc);
    let hdr_bytes = hdr.to_bytes();
    let mut buf = vec![0u8; WalHeader::SIZE + payload.len()];
    buf[..WalHeader::SIZE].copy_from_slice(hdr_bytes);
    buf[WalHeader::SIZE..].copy_from_slice(payload);
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    probe.send_to(&buf, sender_addr).unwrap();

    // Poll until the receiver sees the requested seq. The deadline
    // is a hang-guard only — the fallback's latency is measured by
    // `wal_random_read_bench`, not asserted here.
    sender.recv_control();
    let mut got_seq: Option<u64> = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let CastRecv::Data(rhdr, rpayload) = receiver.try_recv() {
            if rhdr.record_type == RECORD_FILL
                && rpayload.len() >= std::mem::size_of::<FillRecord>()
            {
                let decoded =
                    unsafe { std::ptr::read_unaligned(rpayload.as_ptr() as *const FillRecord) };
                if decoded.seq == target_seq {
                    got_seq = Some(decoded.seq);
                    break;
                }
            }
        }
    }

    assert_eq!(
        got_seq,
        Some(target_seq),
        "WAL-fallback retransmit did not deliver seq={target_seq}",
    );
}
