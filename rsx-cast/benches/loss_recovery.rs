//! Loss recovery cost — how long does casting take to recover a lost packet?
//!
//! Stage 1 (this file): a SINGLE injected gap. A relay between sender and
//! receiver drops exactly one target seq; we time from "the gap becomes
//! detectable" (the next packet is sent) to "the target is delivered". Two
//! tiers, because a record's size decides where the retransmit comes from:
//!
//!   * hot-ring — a small record (≤ 112 B payload) fits a 128 B send-ring
//!     slot, so its retransmit is served from RAM.
//!   * cold-WAL — a `FillRecord` frame is 144 B (128 + 16) and bypasses the
//!     ring, so its retransmit comes from the WAL via `read_record_at_seq`.
//!
//! The NAK repair channel (receiver→sender) is clean, routed to the sender's
//! known bind port; only the data path (through the relay) drops. Public API
//! only, rsx-cast untouched. `harness = false`.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::Framed;
use rsx_cast::wal::WalWriter;
use rsx_cast::CastRecord;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const TRIALS: usize = 25;
const TARGET: u64 = 200; // the seq we drop (well inside the ring/WAL)
const DEADLINE: Duration = Duration::from_secs(10);

/// 64 B record that fits a 128 B send-ring slot → retransmit from RAM.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct SmallRec {
    seq: u64,
    _pad: [u8; 56],
}

impl CastRecord for SmallRec {
    fn seq(&self) -> u64 {
        self.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
    fn record_type() -> u16 {
        100
    }
}

fn small() -> SmallRec {
    SmallRec {
        seq: 0,
        _pad: [0; 56],
    }
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

/// Spawn a relay that forwards relay_addr → recv_addr, dropping exactly the
/// datagram whose record-seq == `drop_seq` (seq is at packet offset 16).
fn spawn_relay(
    relay: UdpSocket,
    recv_addr: SocketAddr,
    drop_seq: u64,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        let mut dropped = false;
        while !stop.load(Ordering::Relaxed) {
            match relay.recv_from(&mut buf) {
                Ok((n, _)) => {
                    if !dropped && n >= 24 {
                        let seq = u64::from_le_bytes(buf[16..24].try_into().unwrap());
                        if seq == drop_seq {
                            dropped = true;
                            continue; // drop this one, once
                        }
                    }
                    let _ = relay.send_to(&buf[..n], recv_addr);
                }
                Err(_) => continue, // read-timeout: re-check stop, keep low latency
            }
        }
    })
}

/// Recovery latency of one dropped `TARGET`. `wal_backed` selects the tier:
/// true → FillRecord persisted to the WAL (cold-WAL retransmit); false →
/// SmallRec ring-cached (hot-ring retransmit).
fn single_gap_recovery(wal_backed: bool) -> Duration {
    let tmp = TempDir::new().unwrap();
    let sender_addr = ephemeral();
    let relay_addr = ephemeral();
    let recv_addr = ephemeral();

    let cfg = CastConfig {
        sender_bind_addr: Some(sender_addr.to_string()),
        heartbeat_interval_ms: 1,
        nak_debounce_us: 50,
        max_nak_retries: 1000,
        ..CastConfig::default()
    };
    let mut sender = CastSender::with_config(relay_addr, 1, tmp.path(), &cfg).unwrap();
    let mut receiver = CastReceiver::with_config(recv_addr, sender_addr, &cfg).unwrap();

    let relay = UdpSocket::bind(relay_addr).unwrap();
    // Blocking recv with a short timeout: wakes the instant a packet lands
    // (low forwarding latency), still checks the stop flag periodically.
    relay
        .set_read_timeout(Some(Duration::from_millis(5)))
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let handle = spawn_relay(relay, recv_addr, TARGET, Arc::clone(&stop));
    thread::sleep(Duration::from_millis(2));

    let mut writer = if wal_backed {
        Some(WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).unwrap())
    } else {
        None
    };
    let mut small_rec = small();
    let mut fill_rec = fill();

    // Send 1..=TARGET (the relay drops TARGET). For the WAL tier, persist +
    // flush so TARGET is on disk before it can be NAK'd.
    let mut send = |seq: u64, sender: &mut CastSender, writer: &mut Option<WalWriter>| {
        if let Some(w) = writer {
            let framed = w.prepare(&mut fill_rec).unwrap();
            debug_assert_eq!(framed.seq, seq);
            w.append_framed(&framed).unwrap();
            sender.send_framed(&framed).unwrap();
        } else {
            let framed = Framed::pack(&mut small_rec, seq);
            sender.send_framed(&framed).unwrap();
        }
    };
    for seq in 1..=TARGET {
        send(seq, &mut sender, &mut writer);
    }
    if let Some(w) = &mut writer {
        w.flush().unwrap();
    }

    // The gap becomes detectable when the receiver sees TARGET+1. Start the
    // clock, send it, then pump recv_control + drain until TARGET is delivered.
    let mut delivered = TARGET - 1;
    let t0 = Instant::now();
    send(TARGET + 1, &mut sender, &mut writer);
    while delivered < TARGET {
        sender.recv_control();
        loop {
            match receiver.try_recv() {
                CastRecv::Data(_, payload) => {
                    let seq = u64::from_le_bytes(payload[0..8].try_into().unwrap());
                    if seq == delivered + 1 {
                        delivered = seq;
                    }
                }
                CastRecv::Empty => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        if t0.elapsed() > DEADLINE {
            panic!("no recovery within {DEADLINE:?}");
        }
    }
    let recovery = t0.elapsed();
    stop.store(true, Ordering::Release);
    let _ = handle.join();
    recovery
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    println!("# casting single-gap recovery latency (median of {TRIALS})\n");
    println!("| tier | record | recover 1 dropped seq |");
    println!("|---|---|---:|");
    for (label, rec, wal) in [
        ("hot-ring", "64 B SmallRec", false),
        ("cold-WAL", "144 B FillRecord", true),
    ] {
        let times: Vec<Duration> = (0..TRIALS).map(|_| single_gap_recovery(wal)).collect();
        let med = median(times);
        println!("| {label} | {rec} | {:.1} µs |", med.as_secs_f64() * 1e6);
    }
}
