//! Outage recovery — how long casting takes to recover after a link outage,
//! and how repeated outages behave as the WAL grows.
//!
//! A relay between sender and receiver goes fully dark for D (drops every
//! datagram), then resumes. The producer WAL-backs every record throughout, so
//! nothing is truly lost — the question is recovery *latency* and *path*. A
//! multi-second outage builds a gap far larger than the 2048 reorder ring, so
//! the receiver returns `Reconnect` and the bench catches up over **TCP
//! replication** (TLS-mandatory) from a `ReplicationService` serving the same
//! WAL, `reset_after_replay`s, and resumes UDP — the real Pattern-A lifecycle.
//!
//! Two scenarios:
//!   A. repeated 1 s outages on one continuous stream (repeated WAL recoveries,
//!      WAL growing + rotating underneath).
//!   B. one 10 s constant outage, then a single recovery.
//!
//! Public API only, rsx-cast untouched. `harness = false`.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::config::TlsClient;
use rsx_cast::config::TlsConfig;
use rsx_cast::config::TlsServer;
use rsx_cast::replication_client::ReplicationConsumer;
use rsx_cast::replication_server::ReplicationService;
use rsx_cast::wal::WalWriter;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;
use tokio::runtime::Runtime;

const PRE: u64 = 500; // clean records before the first outage
const POST_STREAM: u64 = 2500; // > 2048 reorder ring → forces Reconnect
const FLUSH_EVERY: u64 = 256;
const DEADLINE: Duration = Duration::from_secs(90);
// Records/sec assumed to arrive during an outage. The gap that piles up is
// `outage_secs × RATE`; recovery latency is a function of that gap, not of
// real wall-clock. Pacing to a count (rather than firehosing the WAL for the
// full duration) keeps the WAL bounded and the replay path the thing measured.
const RATE: u64 = 5000;

struct Cycle {
    gap: u64,
    recovery: Duration,
}

fn fill() -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: Price(1),
        qty: Qty(1),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    }
}

fn ephemeral() -> SocketAddr {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let a = s.local_addr().unwrap();
    drop(s);
    a
}

fn seq_of(payload: &[u8]) -> u64 {
    u64::from_le_bytes(payload[0..8].try_into().unwrap())
}

/// Self-signed snakeoil certs → (server TlsConfig, client TlsConfig).
fn gen_certs(dir: &Path) -> (TlsConfig, TlsConfig) {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    let server = TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: None,
    };
    let client = TlsConfig {
        server: None,
        client: Some(TlsClient { cert_path }),
    };
    (server, client)
}

/// Apply a sequence of dark-link outages to one continuous stream, recovering
/// (and timing recovery) after each. Returns per-outage cycle results.
fn run_pattern(rt: &Runtime, outages: &[Duration]) -> Vec<Cycle> {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_path_buf();
    let sender_addr = ephemeral();
    let relay_addr = ephemeral();
    let recv_addr = ephemeral();
    let repl_addr = ephemeral();
    let (server_tls, client_tls) = gen_certs(tmp.path());

    // TCP replication service over the producer's WAL (own runtime thread).
    {
        let wal = wal_dir.clone();
        thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            rt.block_on(async move {
                let svc = ReplicationService::new(wal, server_tls).unwrap();
                let _ = svc.serve(repl_addr).await;
            });
        });
    }

    let cfg = CastConfig {
        sender_bind_addr: Some(sender_addr.to_string()),
        heartbeat_interval_ms: 1,
        nak_debounce_us: 200,
        max_nak_retries: 10_000,
        ..CastConfig::default()
    };
    let mut sender = CastSender::with_config(relay_addr, 1, &wal_dir, &cfg).unwrap();
    let mut receiver = CastReceiver::with_config(recv_addr, sender_addr, &cfg).unwrap();
    let mut writer = WalWriter::new(1, &wal_dir, 64 * 1024 * 1024).unwrap();

    // Dark-window relay: forwards relay_addr → recv_addr unless `dark`.
    let relay = UdpSocket::bind(relay_addr).unwrap();
    relay
        .set_read_timeout(Some(Duration::from_millis(2)))
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let dark = Arc::new(AtomicBool::new(false));
    let (stop_r, dark_r) = (Arc::clone(&stop), Arc::clone(&dark));
    let relay_handle = thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while !stop_r.load(Ordering::Relaxed) {
            if let Ok((n, _)) = relay.recv_from(&mut buf) {
                if !dark_r.load(Ordering::Relaxed) {
                    let _ = relay.send_to(&buf[..n], recv_addr);
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(2));

    let mut rec = fill();
    let mut send = |sender: &mut CastSender, writer: &mut WalWriter| -> u64 {
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
        sender.send_framed(&framed).unwrap();
        if framed.seq.is_multiple_of(FLUSH_EVERY) {
            writer.flush().unwrap();
        }
        framed.seq
    };

    // Clean prelude — deliver live.
    let mut delivered = 0u64;
    for _ in 0..PRE {
        send(&mut sender, &mut writer);
        while let CastRecv::Data(_, p) = receiver.try_recv() {
            let s = seq_of(&p);
            if s == delivered + 1 {
                delivered = s;
            }
        }
    }
    writer.flush().unwrap();

    let mut results = Vec::new();
    for &outage in outages {
        // Dark link: produce `outage_secs × RATE` records the receiver never
        // sees (relay drops them), all WAL-backed. This is the gap that must
        // be recovered.
        let before = delivered;
        dark.store(true, Ordering::Release);
        let gap_records = (outage.as_secs_f64() * RATE as f64) as u64;
        let mut frontier = delivered;
        for _ in 0..gap_records {
            frontier = send(&mut sender, &mut writer);
        }
        dark.store(false, Ordering::Release);
        // Phase 3a — burst POST_STREAM contiguous far-ahead records and buffer
        // them into the receiver WITHOUT serving NAKs (no recv_control), so the
        // reorder ring fills (rather than the sender grinding the old gap out
        // seq-by-seq) and overflows → sticky Reconnect. This is what a resumed
        // live stream does after a real outage.
        for _ in 0..POST_STREAM {
            frontier = send(&mut sender, &mut writer);
            let _ = receiver.try_recv();
        }
        writer.flush().unwrap();

        // Phase 3b (measured) — recover the gap. The burst overflowed the
        // reorder ring, so the receiver is in sticky Reconnect. The design
        // response to a gap larger than the ring is NOT to NAK-grind it
        // seq-by-seq (each fill retransmit is a WAL scan — glacial) but to
        // escalate to the TCP replication cold path. We time from fault to
        // caught-up: open a replication consumer, replay delivered+1 → live
        // tip, reset the receiver, resume UDP (the real Pattern-A lifecycle).
        let target = frontier;
        let t0 = Instant::now();
        match receiver.try_recv() {
            CastRecv::Reconnect { .. } | CastRecv::Faulted { .. } => {}
            other => panic!(
                "expected sticky Reconnect after a {POST_STREAM}-record \
                 burst over a {}-record gap, got {other:?}",
                target - before,
            ),
        }
        let tip_file = wal_dir.join("consumer.tip");
        std::fs::write(&tip_file, delivered.to_string()).unwrap();
        let mut consumer =
            ReplicationConsumer::new(1, vec![repl_addr.to_string()], tip_file, client_tls.clone())
                .unwrap();
        rt.block_on(async {
            consumer
                .run_once(|r| {
                    // CaughtUp = the replay reached the live tip; stop so we
                    // can resume UDP. Without this the server tails live
                    // forever and run_once never returns.
                    if r.header.record_type == RECORD_CAUGHT_UP {
                        return false;
                    }
                    let s = seq_of(&r.payload);
                    if s == delivered + 1 {
                        delivered = s;
                    }
                    true
                })
                .await
                .unwrap();
        });
        receiver.reset_after_replay(consumer.tip);
        delivered = delivered.max(consumer.tip);
        let recovery = t0.elapsed();
        assert!(
            delivered >= target,
            "replay fell short: delivered {delivered} < target {target}",
        );
        if t0.elapsed() > DEADLINE {
            panic!("outage {outage:?}: recovery exceeded {DEADLINE:?}");
        }
        eprintln!(
            "  [progress] outage {outage:?}: gap={} recovery={:.0}ms",
            target - before,
            recovery.as_secs_f64() * 1000.0,
        );
        results.push(Cycle {
            gap: target - before,
            recovery,
        });
    }

    stop.store(true, Ordering::Release);
    let _ = relay_handle.join();
    results
}

fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = Runtime::new().unwrap();

    println!("# casting outage recovery\n");

    // Recovery is over TCP replication in every row — a gap this far past the
    // 2048 reorder ring escalates to the cold path rather than NAK-grinding.
    println!("## A — six 1 s outages on one continuous stream\n");
    println!("| # | gap (records) | recovery |");
    println!("|---:|---:|---:|");
    let a = run_pattern(&rt, &[Duration::from_secs(1); 6]);
    for (i, c) in a.iter().enumerate() {
        println!(
            "| {} | {} | {:.1} ms |",
            i + 1,
            c.gap,
            c.recovery.as_secs_f64() * 1000.0,
        );
    }

    println!("\n## B — one 10 s constant outage\n");
    println!("| outage | gap (records) | recovery |");
    println!("|---|---:|---:|");
    let b = run_pattern(&rt, &[Duration::from_secs(10)]);
    for c in &b {
        println!(
            "| 10 s | {} | {:.1} ms |",
            c.gap,
            c.recovery.as_secs_f64() * 1000.0,
        );
    }
}
