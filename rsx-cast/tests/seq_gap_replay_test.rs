//! SEQ-1 regression guard: full gap-detect → FAULTED → TCP
//! replay-from-tip+1 → reset → live-resume loop, end to end.
//!
//! The fixed bug (SEQ-1, 2026-05-29): a producer numbering some
//! records from a second seq counter left a hole in the stream
//! that the receiver read as a gap → false FAULTED. The fix
//! lived in the CALLERS (they now fan out through one seq
//! counter); rsx-cast's recovery path was always sound. This
//! test wires that recovery path end to end so a reintroduced
//! per-stream desync is caught:
//!
//!   1. Receiver consumes a contiguous in-order record (seq=1).
//!   2. A GAP is injected (seq=3 arrives, seq=2 never does) and
//!      idle heartbeats drive the NAK retry budget to
//!      exhaustion → CastRecv::Faulted.
//!   3. A ReplicationService serves the producer's WAL; a
//!      ReplicationConsumer replays from tip+1 (=2) over TCP,
//!      delivering the contiguous tail 2,3.
//!   4. reset_after_replay(3) clears FAULTED.
//!   5. A fresh live record (seq=4) is delivered as Data with
//!      no further fault.
//!
//! If the producers re-desynced their seq numbering, the WAL
//! would carry a hole and the tip+1 replay in step 3 would not
//! deliver a contiguous 2,3 / advance the tip to 3, breaking
//! the reset+resume in steps 4-5.

use rsx_cast::compute_crc32;
use rsx_cast::CastConfig;
use rsx_cast::CastHeartbeat;
use rsx_cast::CastReceiver;
use rsx_cast::CastRecv;
use rsx_cast::ReplicationConsumer;
use rsx_cast::ReplicationService;
use rsx_cast::WalHeader;
use rsx_cast::WalWriter;
use rsx_cast::RECORD_HEARTBEAT;
use rsx_messages::FillRecord;
use rsx_messages::RECORD_FILL;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const STREAM_ID: u32 = 1;

fn reserve_tcp_port() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

/// Self-signed cert used as BOTH server identity and client CA
/// (single-box self-trust). SAN covers 127.0.0.1 for loopback.
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

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000 + seq,
        symbol_id: 1,
        taker_user_id: 10,
        maker_user_id: 20,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
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
    unsafe { std::slice::from_raw_parts(val as *const T as *const u8, std::mem::size_of::<T>()) }
}

fn frame(record_type: u16, payload: &[u8]) -> Vec<u8> {
    let crc = compute_crc32(payload);
    let header = WalHeader::new(record_type, payload.len() as u16, crc);
    let mut buf = vec![0u8; WalHeader::SIZE + payload.len()];
    buf[..WalHeader::SIZE].copy_from_slice(header.to_bytes());
    buf[WalHeader::SIZE..].copy_from_slice(payload);
    buf
}

fn send_fill(probe: &UdpSocket, dest: SocketAddr, seq: u64) {
    let f = fill(seq);
    probe
        .send_to(&frame(RECORD_FILL, as_bytes(&f)), dest)
        .unwrap();
}

fn send_heartbeat(probe: &UdpSocket, dest: SocketAddr, highest_seq: u64) {
    let hb = CastHeartbeat {
        highest_seq,
        _pad1: [0u8; 56],
    };
    probe
        .send_to(&frame(RECORD_HEARTBEAT, as_bytes(&hb)), dest)
        .unwrap();
}

#[test]
fn seq_gap_faults_then_tcp_replay_resumes() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    // Producer's durable WAL: contiguous seqs 1,2,3 — this is
    // the source of truth the TCP replay reads from.
    let mut writer = WalWriter::new(STREAM_ID, &wal_dir, 64 * 1024 * 1024).unwrap();
    for i in 1..=3u64 {
        let mut rec = fill(i);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
    assert_eq!(writer.last_seq(), 3);
    drop(writer);

    // Receiver bound to loopback. Tight fault budget + zero
    // debounce so a small burst of heartbeats exhausts the NAK
    // retries quickly.
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock);
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    let probe_addr = probe.local_addr().unwrap();
    let cfg = CastConfig {
        max_nak_retries: 2,
        nak_debounce_us: 0,
        ..CastConfig::default()
    };
    let mut receiver = CastReceiver::with_config(recv_addr, probe_addr, &cfg).unwrap();

    // 1. Contiguous in-order seq=1 consumes cleanly.
    send_fill(&probe, recv_addr, 1);
    thread::sleep(Duration::from_millis(10));
    match receiver.try_recv() {
        CastRecv::Data(_, p) => {
            let rec = unsafe { std::ptr::read_unaligned(p.as_ptr() as *const FillRecord) };
            assert_eq!(rec.seq, 1);
        }
        other => panic!("expected Data(seq=1), got {other:?}"),
    }

    // 2. GAP: seq=3 arrives, seq=2 never does. Idle heartbeats
    //    claiming highest_seq=3 drive the NAK retry budget to
    //    exhaustion → FAULTED.
    send_fill(&probe, recv_addr, 3);
    thread::sleep(Duration::from_millis(5));
    let _ = receiver.try_recv(); // buffers seq=3, opens gap [2]

    let mut faulted_at = None;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        send_heartbeat(&probe, recv_addr, 3);
        thread::sleep(Duration::from_millis(2));
        if let CastRecv::Faulted {
            last_delivered_seq,
            gap_start,
            ..
        } = receiver.try_recv()
        {
            faulted_at = Some((last_delivered_seq, gap_start));
            break;
        }
    }
    let (last_delivered, gap_start) = faulted_at.expect("receiver should FAULT on the seq=2 gap");
    assert_eq!(last_delivered, 1);
    assert_eq!(gap_start, 2);

    // 3. TCP replay from tip+1. Stand up the replication
    //    service against the producer WAL; consumer tip=1 →
    //    requests from_seq=2, drains the contiguous tail 2,3.
    let tls = test_tls(tmp.path());
    let replay_addr = reserve_tcp_port();
    let wal_dir_srv = wal_dir.clone();
    let tls_srv = tls.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let service = ReplicationService::new(wal_dir_srv, tls_srv).unwrap();
            service.serve(replay_addr).await.unwrap();
        });
    });
    let bind_deadline = Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err() {
        if Instant::now() > bind_deadline {
            panic!("replication server failed to bind");
        }
        thread::sleep(Duration::from_millis(20));
    }

    let tip_file = tmp.path().join("tip.bin");
    std::fs::write(&tip_file, 1u64.to_le_bytes()).unwrap();
    let replayed: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let replayed_cb = replayed.clone();
    let tls_cli = tls.clone();
    let new_tip = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let mut consumer = ReplicationConsumer::new(
                STREAM_ID,
                vec![replay_addr.to_string()],
                tip_file,
                tls_cli,
            )
            .unwrap();
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                consumer.run_once(move |raw| {
                    // Server streams the whole segment; the
                    // consumer dedups by tip (only seq > tip=1
                    // are "new"), same as rsx_risk::drain_replay.
                    if raw.header.record_type == RECORD_FILL {
                        if let Some(seq) = rsx_cast::wal::extract_seq(&raw.payload) {
                            if seq > 1 {
                                replayed_cb.lock().unwrap().push(seq);
                            }
                        }
                    }
                    replayed_cb.lock().unwrap().len() < 2
                }),
            )
            .await;
            consumer.tip
        })
    })
    .join()
    .unwrap();

    assert_eq!(
        replayed.lock().unwrap().clone(),
        vec![2, 3],
        "tip+1 replay must deliver the contiguous tail",
    );
    assert_eq!(new_tip, 3, "replay must advance tip to WAL tip");

    // 4. Reset clears FAULTED, resume point = new_tip+1 = 4.
    receiver.reset_after_replay(new_tip);

    // 5. Fresh live record seq=4 delivers as Data, no fault.
    send_fill(&probe, recv_addr, 4);
    thread::sleep(Duration::from_millis(10));
    match receiver.try_recv() {
        CastRecv::Data(_, p) => {
            let rec = unsafe { std::ptr::read_unaligned(p.as_ptr() as *const FillRecord) };
            assert_eq!(rec.seq, 4, "live stream must resume after replay+reset",);
        }
        other => panic!("expected Data(seq=4) after reset, got {other:?}"),
    }
}
