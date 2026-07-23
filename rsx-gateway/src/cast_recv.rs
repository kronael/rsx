//! Dedicated thread for the risk-\>gateway casting/UDP response stream.
//!
//! GATEWAY-LATENCY (see project BUGS.md): the gateway's WS accept loop,
//! every per-connection handler, and the casting-recv/route step all
//! used to run as cooperatively-scheduled tasks on ONE monoio thread
//! (`GatewayState` is `Rc<RefCell<..>>`, `!Send` by design). Under WS
//! load the casting-recv step competed for scheduler turns with however
//! many connection tasks were runnable, so a risk response could sit
//! unprocessed in the kernel UDP buffer well past when it actually
//! arrived (the `match_done_to_gateway` latency leg).
//!
//! This module moves the recv side off that shared thread entirely: a
//! plain OS thread busy-spins `CastReceiver::try_recv_with` (std
//! non-blocking UDP, no async runtime -- the same pattern rsx-risk's hot
//! loop already uses) and does nothing but decode wire records (all
//! `Copy`, trivially `Send`) into a [`CastEvent`] and push them onto an
//! SPSC ring. `GatewayState` never crosses the thread boundary -- the
//! monoio thread drains the ring (a cheap, non-blocking `pop`, no
//! syscall) and calls the exact same `route_*` functions as before.

use crate::replay::drain_replay;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::decode_payload;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_LIQUIDATION;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use std::env;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use tracing::info;
use tracing::warn;

/// One decoded risk-\>gateway response record, owned so it can cross
/// the thread boundary via the SPSC ring (every variant is `Copy`; the
/// enum derives `Clone, Copy` too, so `push`/`pop` stay allocation-free).
#[derive(Clone, Copy)]
pub enum CastEvent {
    Fill(FillRecord),
    OrderDone(OrderDoneRecord),
    OrderCancelled(OrderCancelledRecord),
    OrderInserted(OrderInsertedRecord),
    OrderFailed(OrderFailedRecord),
    Liquidation(LiquidationRecord),
    ConfigApplied { symbol_id: u32, config_version: u64 },
}

/// Decode one wire record into a [`CastEvent`]. Returns `None` for a
/// record type this path doesn't forward, or a payload too short to
/// decode -- the same tolerance the inline match this replaces had.
pub fn decode_cast_event(record_type: u16, payload: &[u8]) -> Option<CastEvent> {
    match record_type {
        RECORD_FILL => decode_payload::<FillRecord>(payload).map(CastEvent::Fill),
        RECORD_ORDER_DONE => decode_payload::<OrderDoneRecord>(payload).map(CastEvent::OrderDone),
        RECORD_ORDER_CANCELLED => {
            decode_payload::<OrderCancelledRecord>(payload).map(CastEvent::OrderCancelled)
        }
        RECORD_ORDER_INSERTED => {
            decode_payload::<OrderInsertedRecord>(payload).map(CastEvent::OrderInserted)
        }
        RECORD_ORDER_FAILED => {
            decode_payload::<OrderFailedRecord>(payload).map(CastEvent::OrderFailed)
        }
        RECORD_LIQUIDATION => {
            decode_payload::<LiquidationRecord>(payload).map(CastEvent::Liquidation)
        }
        RECORD_CONFIG_APPLIED => {
            decode_payload::<ConfigAppliedRecord>(payload).map(|rec| CastEvent::ConfigApplied {
                symbol_id: rec.symbol_id,
                config_version: rec.config_version,
            })
        }
        _ => None,
    }
}

/// Drain the risk producer's replication stream after the recv thread's
/// cast receiver hit a sticky FAULTED or RECONNECT. Moved here verbatim
/// from the old inline main-loop version (GATEWAY-LATENCY): only the
/// call site's thread changed, not the logic -- `drain_replay` builds
/// its own self-contained tokio runtime, so it has no dependency on
/// which OS thread calls it.
fn handle_replay(last_delivered_seq: u64, gap: Option<(u64, u64)>, wal_dir: &str) -> u64 {
    match gap {
        Some((gs, ge)) => warn!(
            "gateway cast_receiver FAULTED at \
             seq={last_delivered_seq} gap=[{gs}..={ge}], \
             opening replay via RSX_RISK_REPLICATION_ADDR",
        ),
        None => warn!(
            "gateway cast_receiver RECONNECT at \
             seq={last_delivered_seq}, opening replay via \
             RSX_RISK_REPLICATION_ADDR",
        ),
    }
    let replay_addr = match env::var("RSX_RISK_REPLICATION_ADDR") {
        Ok(a) => a,
        Err(_) => {
            let skip_to = gap.map(|(_, ge)| ge).unwrap_or(last_delivered_seq);
            warn!(
                "RSX_RISK_REPLICATION_ADDR not set; \
                 skipping gap to seq={skip_to} (in-flight fills lost)"
            );
            return skip_to;
        }
    };
    // Gateway sees a merged stream from risk (response side);
    // stream_id 0 matches `CastSender::new(.., 0, ..)` on the
    // risk gw_sender.
    let tip_file = PathBuf::from(wal_dir).join("gateway_replay_tip.bin");
    let tls = rsx_cast::TlsConfig::from_env()
        .unwrap_or_else(|e| panic!("gateway replay requires TLS: {e}"));
    let new_tip = drain_replay(0, replay_addr.clone(), last_delivered_seq, tip_file, tls, |raw| {
        let seq = rsx_cast::wal::extract_seq(&raw.payload).unwrap_or(0);
        tracing::debug!(
            "gateway replay applied record_type={} seq={}",
            raw.header.record_type,
            seq,
        );
    })
    .unwrap_or_else(|e| {
        panic!("gateway replay drain failed against {replay_addr}: {e}",)
    });
    info!("gateway replay drained, new_tip={new_tip}, resuming",);
    new_tip
}

/// Bind a loopback UDP pair used purely as a cross-thread wake signal:
/// the recv thread pings `send` after each drained batch; the monoio
/// thread dups `recv`'s fd into its own reactor for a `readable()` wait
/// (exactly the technique already used to turn the cast fd into a
/// monoio readiness source), and drains `recv` itself on wake. Both
/// ends are non-blocking -- a full send buffer or a spurious miss just
/// means the next batch's ping (or the housekeeping timer) catches up.
fn doorbell_pair() -> std::io::Result<(UdpSocket, UdpSocket)> {
    let recv = UdpSocket::bind("127.0.0.1:0")?;
    recv.set_nonblocking(true)?;
    let addr = recv.local_addr()?;
    let send = UdpSocket::bind("127.0.0.1:0")?;
    send.set_nonblocking(true)?;
    send.connect(addr)?;
    Ok((recv, send))
}

/// Handle returned by [`spawn`]: the SPSC consumer the monoio thread
/// drains every loop iteration, the doorbell socket it wraps for a
/// readiness wait, and the thread's `JoinHandle` -- joined on shutdown
/// (mirrors rsx-risk's persist-thread shutdown/join pattern).
pub struct CastRecvHandle {
    pub consumer: rtrb::Consumer<CastEvent>,
    pub doorbell: UdpSocket,
    pub join: JoinHandle<()>,
}

/// Spawn the dedicated casting-recv thread. `cast_receiver` and
/// `wal_dir` are exactly what the old inline loop owned; `shutdown` is
/// the same process-wide flag `install_shutdown_handler` returns.
pub fn spawn(
    cast_receiver: CastReceiver,
    wal_dir: String,
    shutdown: &'static AtomicBool,
    ring_capacity: usize,
) -> std::io::Result<CastRecvHandle> {
    let (doorbell_recv, doorbell_send) = doorbell_pair()?;
    let (producer, consumer) = rtrb::RingBuffer::<CastEvent>::new(ring_capacity);
    let join = std::thread::Builder::new()
        .name("gw-cast-recv".into())
        .spawn(move || run(cast_receiver, wal_dir, shutdown, producer, doorbell_send))?;
    Ok(CastRecvHandle {
        consumer,
        doorbell: doorbell_recv,
        join,
    })
}

/// The recv thread's body: busy-spin `try_recv_with`, decode, push,
/// ping the doorbell once per drained batch. Never touches
/// `GatewayState` -- routing stays on the monoio thread.
fn run(
    mut cast_receiver: CastReceiver,
    wal_dir: String,
    shutdown: &'static AtomicBool,
    mut producer: rtrb::Producer<CastEvent>,
    doorbell: UdpSocket,
) {
    while !shutdown.load(Ordering::Relaxed) {
        let mut delivered = false;
        loop {
            let outcome = cast_receiver.try_recv_with(|hdr, payload| {
                let Some(event) = decode_cast_event(hdr.record_type, payload) else {
                    return;
                };
                // `rec` is only read by `latency_sample!`'s body, which
                // compiles away entirely without the `latency-trace`
                // feature (rsx-log/src/lib.rs) -- allow the resulting
                // unused-binding lint on the default (feature-off) build.
                #[allow(unused_variables)]
                if let CastEvent::Fill(rec) = &event {
                    // Sub-stage: the recv thread saw this fill -- now a
                    // true "did the response arrive" timestamp, no
                    // longer entangled with the monoio reactor's
                    // scheduling of the old inline recv-and-route step.
                    rsx_log::latency_sample!(
                        "gateway_cast_recv",
                        rec.taker_order_id_hi,
                        rec.taker_order_id_lo,
                        if rec.gw_in_ns > 1_700_000_000_000_000_000 {
                            rec.gw_in_ns
                        } else {
                            rec.ts_ns
                        }
                    );
                }
                // Ring full = producer stalls (project convention,
                // mirrors rsx-risk's egress rings): spin rather than
                // drop a response. The monoio thread drains every loop
                // iteration, so this only blocks a truly wedged
                // consumer -- checked against shutdown so a stuck
                // monoio thread can't hang process exit.
                let mut pending = event;
                while let Err(rtrb::PushError::Full(returned)) = producer.push(pending) {
                    pending = returned;
                    if shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    std::hint::spin_loop();
                }
            });
            match outcome {
                CastRecvWith::Data => delivered = true,
                CastRecvWith::Empty => break,
                CastRecvWith::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => {
                    let new_tip = handle_replay(
                        last_delivered_seq,
                        Some((gap_start, gap_end_inclusive)),
                        &wal_dir,
                    );
                    cast_receiver.reset_after_replay(new_tip);
                }
                CastRecvWith::Reconnect { last_delivered_seq } => {
                    let new_tip = handle_replay(last_delivered_seq, None, &wal_dir);
                    cast_receiver.reset_after_replay(new_tip);
                }
            }
        }
        if delivered {
            // Best-effort: a dropped ping just means the monoio thread
            // catches up on the next housekeeping timer tick instead of
            // instantly -- never fatal, never worth retrying here.
            let _ = doorbell.send(&[0u8; 1]);
        }
    }
}

#[cfg(test)]
#[path = "cast_recv_test.rs"]
mod cast_recv_test;
