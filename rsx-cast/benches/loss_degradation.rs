//! Loss tolerance: how much sustained packet loss casting's live NAK recovery
//! survives, and how throughput degrades approaching that ceiling.
//!
//! A userspace lossy UDP relay sits between `CastSender` and `CastReceiver`
//! and drops each forwarded datagram — first transmissions AND retransmits —
//! with probability `loss`. Dropping retransmits too is the whole point: a gap
//! is only unrecoverable when the same seq is dropped `max_nak_retries` times
//! in a row (~`loss^retries`), so the ceiling is a function of the retry
//! budget, and retransmits must be allowed to fail to find it. The NAK repair
//! channel (receiver→sender) does NOT cross the relay — it routes directly to
//! the sender's known bind port — so this is a one-way-lossy forward link, the
//! realistic case (the NAK side of a link is rarely the congested one).
//!
//! Records are 64 B, so they fit a 128 B send-ring slot and retransmits are
//! served from RAM in ~µs (no WAL scan). That isolates the NAK protocol's loss
//! tolerance from the O(N) cold-WAL retransmit cost that 144 B fills pay (see
//! `loss_recovery` for that tier). Config is the DEFAULT 8-retry budget with a
//! short loopback debounce; a gap therefore dies after 8 consecutive
//! retransmit losses, and the sweep finds the loss rate where that bites.
//!
//! Flow-controlled to `WINDOW` outstanding records (< the 2048 reorder ring)
//! so out-of-order buffering never overflows into the `Reconnect` cold path.
//!
//! Public API only, rsx-cast untouched. `harness = false`.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::Framed;
use rsx_cast::CastRecord;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const N: u64 = 10_000;
const TRIALS: usize = 5;
const WINDOW: u64 = 1024; // outstanding records < 2048 reorder ring
const DEADLINE: Duration = Duration::from_secs(30);

/// 64 B record → fits a 128 B send-ring slot → retransmit from RAM.
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

/// Outcome of one trial: delivered all N in order, or the live path gave up on
/// a gap (`Faulted` = retry budget exhausted, or `Reconnect` = ring overflow —
/// both mean live NAK recovery couldn't sustain this loss), or timed out.
enum Outcome {
    Delivered(Duration),
    LivePathGaveUp,
    Timeout,
}

// xorshift64 — deterministic per-trial loss pattern, no rand dep.
fn xorshift(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}

fn ephemeral_port() -> u16 {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let p = s.local_addr().unwrap().port();
    drop(s);
    p
}

/// Stream N records through a relay dropping `loss` of every datagram; return
/// the outcome. `seed` fixes the loss pattern for reproducibility.
fn run_trial(loss: f64, seed: u64) -> Outcome {
    let tmp = TempDir::new().unwrap();
    let sender_addr: SocketAddr = format!("127.0.0.1:{}", ephemeral_port()).parse().unwrap();
    let relay_addr: SocketAddr = format!("127.0.0.1:{}", ephemeral_port()).parse().unwrap();
    let recv_addr: SocketAddr = format!("127.0.0.1:{}", ephemeral_port()).parse().unwrap();

    // Default 8-retry budget. The debounce must exceed a worst-case
    // retransmit round-trip (including scheduling jitter on a loaded box),
    // else a merely-late — not lost — retransmit burns a retry and faults
    // spuriously. 5 ms clears loopback ring-retransmit latency with margin;
    // the ceiling is set by the retry COUNT (8), not the debounce, so this
    // only affects speed, not tolerance. Sender binds a known port so NAKs
    // route back directly, off the lossy relay.
    let cfg = CastConfig {
        sender_bind_addr: Some(sender_addr.to_string()),
        heartbeat_interval_ms: 1,
        nak_debounce_us: 5_000,
        max_nak_retries: 8,
        ..CastConfig::default()
    };
    let mut sender = CastSender::with_config(relay_addr, 1, tmp.path(), &cfg).unwrap();
    let mut receiver = CastReceiver::with_config(recv_addr, sender_addr, &cfg).unwrap();

    // Lossy relay: relay_addr -> recv_addr, Bernoulli-drop every datagram
    // (first tx and retransmits alike) with probability `loss`.
    // Blocking recv with a short timeout: wakes the instant a packet lands
    // (low forwarding latency, so retransmits are prompt) without burning a
    // core busy-spinning — core contention is what inflates retransmit jitter.
    let relay = UdpSocket::bind(relay_addr).unwrap();
    relay
        .set_read_timeout(Some(Duration::from_millis(1)))
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_r = Arc::clone(&stop);
    let relay_handle = thread::spawn(move || {
        let mut buf = [0u8; 2048];
        let mut rng = seed | 1;
        while !stop_r.load(Ordering::Relaxed) {
            if let Ok((n, _)) = relay.recv_from(&mut buf) {
                let r = (xorshift(&mut rng) >> 11) as f64 / (1u64 << 53) as f64;
                if r >= loss {
                    let _ = relay.send_to(&buf[..n], recv_addr);
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(2)); // let the relay spin up

    let mut rec = SmallRec {
        seq: 0,
        _pad: [0; 56],
    };
    let start = Instant::now();
    let mut send_seq = 0u64;
    let mut delivered = 0u64;
    let mut outcome = Outcome::Timeout;
    'trial: while delivered < N {
        if send_seq < N && send_seq.saturating_sub(delivered) < WINDOW {
            send_seq += 1;
            let framed = Framed::pack(&mut rec, send_seq);
            sender.send_framed(&framed).unwrap();
        } else {
            // Stalled on a gap (or all sent): emit heartbeats so the receiver
            // keeps NAKing the missing seq.
            let _ = sender.tick();
        }
        sender.recv_control(); // serve NAKs (retransmit from the send ring)
        loop {
            match receiver.try_recv() {
                CastRecv::Data(_, payload) => {
                    let seq = u64::from_le_bytes(payload[0..8].try_into().unwrap());
                    if seq == delivered + 1 {
                        delivered = seq;
                    }
                }
                CastRecv::Empty => break,
                // The live path gave up on a gap: this loss rate is past the
                // ceiling. Record it and stop (the sweep wants the threshold,
                // not a crash).
                CastRecv::Faulted { .. } | CastRecv::Reconnect { .. } => {
                    outcome = Outcome::LivePathGaveUp;
                    break 'trial;
                }
            }
        }
        if start.elapsed() > DEADLINE {
            outcome = Outcome::Timeout;
            break 'trial;
        }
    }
    if delivered >= N {
        outcome = Outcome::Delivered(start.elapsed());
    }
    stop.store(true, Ordering::Release);
    let _ = relay_handle.join();
    outcome
}

fn main() {
    println!("# casting loss tolerance ({N} records in order, {TRIALS} trials/rate)\n");
    println!(
        "One-way-lossy forward link (retransmits drop too); 64 B ring-served \
         records; default 8-retry budget, loopback-fast debounce; \
         flow-controlled under the reorder ring. `delivered` = trials that \
         delivered all {N} in order; a gap that outruns 8 consecutive \
         retransmit losses makes the live path give up (→ TCP-replay \
         territory, out of scope here).\n"
    );
    println!("| loss rate | delivered | median throughput | vs 0-loss |");
    println!("|---|---:|---:|---:|");
    let mut base_s: Option<f64> = None;
    for &loss in &[0.0f64, 0.01, 0.05, 0.10, 0.20, 0.25, 0.30, 0.40] {
        let mut oks: Vec<Duration> = Vec::new();
        let mut gaveup = 0usize;
        let mut timeouts = 0usize;
        for t in 0..TRIALS {
            match run_trial(loss, 0x9E3779B97F4A7C15 ^ (t as u64 + 1)) {
                Outcome::Delivered(d) => oks.push(d),
                Outcome::LivePathGaveUp => gaveup += 1,
                Outcome::Timeout => timeouts += 1,
            }
        }
        let note = if gaveup == 0 && timeouts == 0 {
            format!("{TRIALS}/{TRIALS}")
        } else {
            format!("{}/{TRIALS} ({}gaveup {}T)", oks.len(), gaveup, timeouts)
        };
        if oks.is_empty() {
            println!("| {:.0}% | {} | — | — |", loss * 100.0, note);
            continue;
        }
        oks.sort();
        let secs = oks[oks.len() / 2].as_secs_f64();
        let thpt = N as f64 / secs;
        let base = *base_s.get_or_insert(secs);
        println!(
            "| {:.0}% | {} | {:.0} rec/s | {:.2}× |",
            loss * 100.0,
            note,
            thpt,
            secs / base,
        );
    }
}
