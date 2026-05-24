//! WAL random-access NAK fallback latency.
//!
//! Send enough records to push past the send-ring horizon
//! (SEND_RING_CAPACITY = 4096), then NAK an early seq that
//! the ring no longer holds. The retransmit must come from
//! disk via `read_record_at_seq`. Assert wall-clock budget.
//!
//! Completes in ~70ms on a typical dev box; runs as a normal
//! test under `make test`.

use rsx_cast::cmp::CmpRecv;
use rsx_cast::cmp::CmpReceiver;
use rsx_cast::cmp::CmpSender;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::protocol::Nak;
use rsx_cast::protocol::RECORD_NAK;
use rsx_cast::wal::WalWriter;
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

fn as_bytes<T>(val: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            val as *const T as *const u8,
            std::mem::size_of::<T>(),
        )
    }
}

#[test]
fn nak_wal_fallback_under_5ms() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path();

    // Populate WAL with records 1..=5000 so they survive on
    // disk after the in-memory send ring has wrapped past
    // them. SEND_RING_CAPACITY is 4096.
    let stream_id = 1u32;
    let mut writer = WalWriter::new(
        stream_id,
        wal_dir,
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    for i in 1..=5_000u64 {
        let mut rec = fill(i);
        writer.append(&mut rec).unwrap();
    }
    writer.flush().unwrap();

    // Build sender/receiver loopback.
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock);
    let mut sender =
        CmpSender::new(recv_addr, stream_id, wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver =
        CmpReceiver::new(recv_addr, sender_addr, stream_id)
            .unwrap();

    // Push 5000 sends through the in-memory ring so the
    // sender's `ring_seqs[slot for seq=1]` has been
    // overwritten (slot is (seq & 4095)). The seq returned
    // by the sender now starts at 1 — fine, since the WAL we
    // wrote uses the same convention. Drain receiver each
    // batch so its reorder buffer doesn't fill.
    for _ in 0..5_000u64 {
        let mut rec = fill(0);
        sender.send(&mut rec).unwrap();
        // Drain occasionally to avoid OS UDP buffer overruns.
        while matches!(
            receiver.try_recv(),
            CmpRecv::Data(_, _)
        ) {}
    }
    // Final drain.
    thread::sleep(Duration::from_millis(20));
    while matches!(
        receiver.try_recv(),
        CmpRecv::Data(_, _)
    ) {}

    // Issue a NAK from a third-party socket for seq=1 — the
    // ring's slot for seq=1 has been overwritten by seq=4097
    // (1 + 4096) by now, so the sender must fall through to
    // WAL.
    let target_seq = 1u64;
    let nak = Nak {
        from_seq: target_seq,
        count: 1,
        _pad1: [0u8; 48],
    };
    let payload = as_bytes(&nak);
    let crc = compute_crc32(payload);
    let hdr = WalHeader::new(
        RECORD_NAK,
        payload.len() as u16,
        crc,
    );
    let hdr_bytes = hdr.to_bytes();
    let mut buf = vec![0u8; WalHeader::SIZE + payload.len()];
    buf[..WalHeader::SIZE].copy_from_slice(&hdr_bytes);
    buf[WalHeader::SIZE..].copy_from_slice(payload);
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    probe.send_to(&buf, sender_addr).unwrap();

    // Time only the retransmit path: poll until the receiver
    // sees the requested seq.
    let t0 = Instant::now();
    sender.recv_control();

    let mut got_seq: Option<u64> = None;
    let deadline = t0 + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let CmpRecv::Data(rhdr, rpayload) =
            receiver.try_recv()
        {
            if rhdr.record_type == RECORD_FILL
                && rpayload.len()
                    >= std::mem::size_of::<FillRecord>()
            {
                let decoded = unsafe {
                    std::ptr::read_unaligned(
                        rpayload.as_ptr()
                            as *const FillRecord,
                    )
                };
                if decoded.seq == target_seq {
                    got_seq = Some(decoded.seq);
                    break;
                }
            }
        }
    }
    let elapsed = t0.elapsed();

    assert_eq!(
        got_seq,
        Some(target_seq),
        "WAL-fallback retransmit did not deliver seq={target_seq}",
    );
    assert!(
        elapsed < Duration::from_millis(5),
        "WAL-fallback retransmit took {elapsed:?}, \
         expected < 5ms (warm cache budget)",
    );
}
