//! Dedicated thread for the risk-\>gateway casting/UDP response stream.
//!
//! A plain OS thread busy-spins `CastReceiver::try_recv_with`, decodes
//! wire records into [`CastEvent`], and pushes them onto an SPSC ring.
//! `GatewayState` remains on the monoio thread, which drains and routes
//! the events.

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
/// decode.
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

/// Queue `event` for the monoio thread. Ring full = producer stalls
/// (project convention, mirrors rsx-risk's egress rings): spin rather
/// than drop a response. The monoio thread drains every loop iteration,
/// so this only blocks a truly wedged consumer -- checked against
/// shutdown so a stuck monoio thread can't hang process exit.
fn push_event(producer: &mut rtrb::Producer<CastEvent>, shutdown: &AtomicBool, event: CastEvent) {
    let mut pending = event;
    while let Err(rtrb::PushError::Full(returned)) = producer.push(pending) {
        pending = returned;
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        std::hint::spin_loop();
    }
}

/// Fetch the records of a FAULTED or RECONNECT gap from risk's replication
/// server at `replay_addr`, decode each with [`decode_cast_event`] and push
/// it on `producer`, the ring live records take, so the monoio thread routes
/// a replayed record exactly like a live one. Returns the new tip. A failed
/// drain warns and returns `skip_to` instead: the gap is lost, the gateway
/// keeps running.
pub fn replay_gap(
    replay_addr: &str,
    tls: rsx_cast::TlsConfig,
    tip_file: PathBuf,
    last_delivered_seq: u64,
    skip_to: u64,
    shutdown: &AtomicBool,
    producer: &mut rtrb::Producer<CastEvent>,
) -> u64 {
    // Stream 0 matches `CastSender::new(.., 0, ..)` on risk's gateway sender.
    let drained = drain_replay(
        0,
        replay_addr.to_string(),
        last_delivered_seq,
        tip_file,
        tls,
        |raw| {
            if let Some(event) = decode_cast_event(raw.header.record_type, &raw.payload) {
                push_event(producer, shutdown, event);
            }
        },
    );
    drained.unwrap_or_else(|e| {
        warn!(
            "gateway replay drain failed against {replay_addr}: {e}; \
             skipping gap to seq={skip_to} (in-flight fills lost)"
        );
        skip_to
    })
}

/// Recover after the recv thread's cast receiver hit a sticky FAULTED or
/// RECONNECT: replay from risk via `RSX_RISK_REPLICATION_ADDR`, or skip the
/// gap when the replay cannot run. Returns the seq to resume after.
fn handle_replay(
    last_delivered_seq: u64,
    gap: Option<(u64, u64)>,
    wal_dir: &str,
    shutdown: &AtomicBool,
    producer: &mut rtrb::Producer<CastEvent>,
) -> u64 {
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
    let skip_to = gap.map(|(_, ge)| ge).unwrap_or(last_delivered_seq);
    let Ok(replay_addr) = env::var("RSX_RISK_REPLICATION_ADDR") else {
        warn!(
            "RSX_RISK_REPLICATION_ADDR not set; \
             skipping gap to seq={skip_to} (in-flight fills lost)"
        );
        return skip_to;
    };
    let tls = match rsx_cast::TlsConfig::from_env() {
        Ok(tls) => tls,
        Err(e) => {
            warn!(
                "gateway replay requires TLS: {e}; \
                 skipping gap to seq={skip_to} (in-flight fills lost)"
            );
            return skip_to;
        }
    };
    let tip_file = PathBuf::from(wal_dir).join("gateway_replay_tip.bin");
    replay_gap(
        &replay_addr,
        tls,
        tip_file,
        last_delivered_seq,
        skip_to,
        shutdown,
        producer,
    )
}

/// Bind a loopback UDP pair used purely as a cross-thread wake signal:
/// the recv thread pings `send` after each drained batch; the monoio
/// thread dups `recv`'s fd into its own reactor for a `readable()` wait
/// and drains `recv` itself on wake. Both
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
/// readiness wait, and the thread's `JoinHandle`, joined on shutdown.
pub struct CastRecvHandle {
    pub consumer: rtrb::Consumer<CastEvent>,
    pub doorbell: UdpSocket,
    pub join: JoinHandle<()>,
}

/// Spawn the dedicated casting-recv thread. It pins itself to `core_id`
/// before it starts spinning; `None` leaves it unpinned.
pub fn spawn(
    cast_receiver: CastReceiver,
    wal_dir: String,
    shutdown: &'static AtomicBool,
    ring_capacity: usize,
    core_id: Option<usize>,
) -> std::io::Result<CastRecvHandle> {
    let (doorbell_recv, doorbell_send) = doorbell_pair()?;
    let (producer, consumer) = rtrb::RingBuffer::<CastEvent>::new(ring_capacity);
    let join = std::thread::Builder::new()
        .name("gw-cast-recv".into())
        .spawn(move || {
            pin(core_id);
            run(cast_receiver, wal_dir, shutdown, producer, doorbell_send)
        })?;
    Ok(CastRecvHandle {
        consumer,
        doorbell: doorbell_recv,
        join,
    })
}

/// Pin the calling thread to `core_id` and log what that achieved.
fn pin(core_id: Option<usize>) {
    let Some(core_id) = core_id else {
        warn!("RSX_GW_CAST_CORE_ID not set: gw-cast-recv busy-spin thread is not pinned");
        return;
    };
    let setup = rsx_types::cpu::setup_hot_thread(core_id);
    info!("gateway cast-recv {}", setup);
    if !setup.pinned {
        warn!("gateway cast-recv could not pin to core {core_id}");
    } else if setup.isolated == Some(false) {
        warn!("gateway core {core_id} not isolated — expect tail spikes");
    }
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
                // Read only by `latency_sample!`, which compiles away
                // without the `latency-trace` feature.
                #[allow(unused_variables)]
                if let CastEvent::Fill(rec) = &event {
                    rsx_log::latency_sample!(
                        "gateway_cast_recv",
                        rec.taker_order_id_hi,
                        rec.taker_order_id_lo,
                        crate::route::latency_anchor_ns(rec)
                    );
                }
                push_event(&mut producer, shutdown, event);
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
                        shutdown,
                        &mut producer,
                    );
                    cast_receiver.reset_after_replay(new_tip);
                    delivered = true;
                }
                CastRecvWith::Reconnect { last_delivered_seq } => {
                    let new_tip =
                        handle_replay(last_delivered_seq, None, &wal_dir, shutdown, &mut producer);
                    cast_receiver.reset_after_replay(new_tip);
                    delivered = true;
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
