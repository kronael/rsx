//! The live loop: one pinned thread draining the ME, gateway and
//! mark cast receivers straight into `RiskShard`, forwarding ME
//! records to the gateway and accepted orders to their ME. Std
//! non-blocking UDP busy-spin; no async runtime on this thread.

use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::decode_payload;
use rsx_cast::TlsConfig;
use rsx_health::LoadGauges;
use rsx_messages::BboRecord;
use rsx_messages::CancelRequest;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::MarkPriceRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::OrderMessage;
use rsx_messages::OrderRequest;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_MARK_PRICE;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_risk::config::RuntimeConfig;
use rsx_risk::replay::apply_me_replay_record;
use rsx_risk::replay::handle_replay;
use rsx_risk::rings::send_to_me;
use rsx_risk::rings::GwStream;
use rsx_risk::rings::MeCommand;
use rsx_risk::rings::ShardRings;
use rsx_risk::shard::RiskShard;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::OrderResponse;
use rsx_types::time_utils::time;
use rsx_types::time_utils::time_ns;
use rsx_types::FailureReason;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::Instant;
use tracing::info;
use tracing::warn;

struct MeReceiverState {
    symbol_id: u32,
    replay_addr: String,
    receiver: CastReceiver,
}

/// Every cast socket the live loop drives. `mark_receiver` is bound
/// before warm catch-up and carries on into the live loop.
pub struct CastIo {
    me_receivers: Vec<MeReceiverState>,
    gw_receiver: CastReceiver,
    mark_receiver: CastReceiver,
    me_senders: HashMap<u32, CastSender>,
    gw_stream: GwStream,
}

/// Why the live loop returned. The caller owns the persist worker
/// and lease thread and tears them down on every variant.
pub enum LiveExit {
    Shutdown,
    PersistWorkerDied,
    LeaseLost,
    LeaseError,
}

/// Bind the gateway and ME receivers and create one sender per ME
/// plus the gateway sender. Fails fast: a socket that cannot bind at
/// promotion is a topology error.
pub fn bind(cfg: &RuntimeConfig, mark_receiver: CastReceiver, wal_dir: &str) -> CastIo {
    let gw_receiver = CastReceiver::new(cfg.risk_addr, cfg.gw_sender_addr)
        .expect("failed to bind risk cast receiver");

    let mut me_receivers = Vec::with_capacity(cfg.me_streams.len());
    for stream in &cfg.me_streams {
        let receiver = CastReceiver::new(stream.recv_addr, stream.sender_addr)
            .expect("failed to bind ME fill receiver");
        me_receivers.push(MeReceiverState {
            symbol_id: stream.symbol_id,
            replay_addr: stream.replay_addr.clone(),
            receiver,
        });
    }

    // Send validated orders to ME.
    // One CastSender per ME, keyed by symbol_id.
    let mut me_senders: HashMap<u32, CastSender> = HashMap::new();
    for stream in &cfg.me_streams {
        let me_sender_cfg = CastConfig {
            sender_bind_addr: stream.send_bind.clone(),
            ..Default::default()
        };
        let sender =
            CastSender::with_config(stream.cast_addr, 0, Path::new(wal_dir), &me_sender_cfg)
                .expect("failed to create ME cast sender");
        me_senders.insert(stream.symbol_id, sender);
    }

    // Send responses to Gateway
    let gw_sender_cfg = CastConfig {
        sender_bind_addr: cfg.gw_send_bind.clone(),
        ..Default::default()
    };
    let gw_stream = GwStream::open(cfg.gw_addr, Path::new(wal_dir), &gw_sender_cfg)
        .expect("failed to open GW stream: WAL or cast sender");

    CastIo {
        me_receivers,
        gw_receiver,
        mark_receiver,
        me_senders,
        gw_stream,
    }
}

/// Busy-spin until shutdown, lease loss or persist-worker death.
/// Priority per pass: ME events, gateway orders, mark prices, then
/// `RiskShard::tick`, then egress drains.
#[allow(clippy::too_many_arguments)]
pub fn run(
    shard: &mut RiskShard,
    io: &mut CastIo,
    cfg: &RuntimeConfig,
    tls: &TlsConfig,
    gauges: &LoadGauges,
    shutdown: &AtomicBool,
    lease_held: &AtomicBool,
    lease_error: &AtomicBool,
    persist_handle: &JoinHandle<()>,
    shard_id: u32,
) -> LiveExit {
    let wal_dir: &str = cfg
        .wal_dir
        .to_str()
        .expect("RSX_RISK_WAL_DIR must be valid UTF-8");

    // Egress SPSC rings: the recv loop calls shard.process_*
    // directly (same thread); these two carry shard output back
    // to this loop's casting senders.
    let (resp_prod, mut resp_cons) = rtrb::RingBuffer::<OrderResponse>::new(2048);
    let (me_prod, mut me_cons) = rtrb::RingBuffer::<MeCommand>::new(2048);
    gauges.resp_ring_cap.store(2048, Ordering::Relaxed);
    gauges.accept_ring_cap.store(2048, Ordering::Relaxed);

    let mut rings = ShardRings {
        response_producer: resp_prod,
        me_producer: me_prod,
    };

    let CastIo {
        me_receivers,
        gw_receiver,
        mark_receiver,
        me_senders,
        gw_stream,
    } = io;

    const CLOCK_REFRESH_SPINS: u32 = 1024;
    let mut clock = Instant::now();
    let mut spin_count: u32 = 0;

    loop {
        if spin_count.is_multiple_of(CLOCK_REFRESH_SPINS) {
            clock = Instant::now();
        }
        spin_count = spin_count.wrapping_add(1);

        // Graceful shutdown: stop accepting new work; the caller
        // drains the persist ring to Postgres (durable
        // positions/freezes), stops the lease thread, and exits
        // directly so the node never re-enters warm catchup.
        if shutdown.load(Ordering::SeqCst) {
            info!("shutdown signal received, draining persist ring to postgres");
            if let Err(e) = gw_stream.flush() {
                warn!("risk: gw wal flush at shutdown failed: {e}");
            }
            return LiveExit::Shutdown;
        }

        let now_secs = time();

        // Events from ME (fills, BBO, order lifecycle) are
        // drained BEFORE orders so an order's pre-trade margin
        // check sees margin freed by this tick's fills/releases
        // (capital efficiency; fills-before-orders invariant).
        for stream in me_receivers.iter_mut() {
            let recv = stream.receiver.try_recv_with(|hdr, payload| {
                match hdr.record_type {
                    RECORD_BBO => {
                        if let Some(rec) = decode_payload::<BboRecord>(payload) {
                            // BBO is a "latest wins" state snapshot.
                            // Stash (coalesces per symbol); the tick
                            // drains + runs the per-BBO margin scan.
                            shard.stash_bbo(BboUpdate::from(&rec));
                            // BBO is consumed only by risk (margin scan). It is
                            // NOT forwarded to the gateway: the gateway has no BBO
                            // handler, and the gateway stream numbers its own
                            // records, so dropping it leaves no seq gap. Public BBO
                            // reaches clients via the marketdata process.
                        }
                    }
                    RECORD_FILL => {
                        if let Some(mut fill) = decode_payload::<FillRecord>(payload) {
                            // `risk_out` records a fill received from
                            // ME and about to be forwarded to gateway.
                            // Anchor against the taker's gateway-ingress
                            // timestamp (fill.gw_in_ns) so this delta
                            // composes with gateway_in / risk_in / me_in
                            // on the same clock origin. Fall back to
                            // ts_ns when the field is zero.
                            rsx_log::latency_sample!(
                                "risk_out",
                                fill.taker_order_id_hi,
                                fill.taker_order_id_lo,
                                if fill.gw_in_ns == 0 {
                                    fill.ts_ns
                                } else {
                                    fill.gw_in_ns
                                }
                            );
                            // Fills are correctness-critical:
                            // position == sum(fills). Process
                            // directly on this thread (no input
                            // ring) — process_fill only touches
                            // shard state and the persist ring,
                            // which has its own stall path.
                            shard.process_fill(&FillEvent::from(&fill));
                            gauges.fills_processed.fetch_add(1, Ordering::Relaxed);
                            gw_stream.forward(&mut fill);
                            // Sub-stage: cast send to gateway completed.
                            // Anchor on the same gw_in_ns used by
                            // risk_out (with the >2024 plausibility
                            // guard).
                            rsx_log::latency_sample!(
                                "risk_cast_send_done",
                                fill.taker_order_id_hi,
                                fill.taker_order_id_lo,
                                if fill.gw_in_ns == 0 {
                                    fill.ts_ns
                                } else {
                                    fill.gw_in_ns
                                }
                            );
                        }
                    }
                    RECORD_ORDER_DONE => {
                        if let Some(mut rec) = decode_payload::<OrderDoneRecord>(payload) {
                            shard.release_frozen_for_order(
                                rec.user_id,
                                rec.order_id_hi,
                                rec.order_id_lo,
                            );
                            gw_stream.forward(&mut rec);
                        }
                    }
                    RECORD_ORDER_INSERTED => {
                        if let Some(mut rec) = decode_payload::<OrderInsertedRecord>(payload) {
                            gw_stream.forward(&mut rec);
                        }
                    }
                    RECORD_ORDER_ACCEPTED => {
                        if let Some(rec) = decode_payload::<OrderAcceptedRecord>(payload) {
                            // ME confirmed the order: now (and only now)
                            // write the durable freeze. Reduce-only orders
                            // reserve no margin, so nothing to persist.
                            if rec.reduce_only == 0 {
                                shard.confirm_freeze(
                                    rec.user_id,
                                    rec.order_id_hi,
                                    rec.order_id_lo,
                                    rec.symbol_id,
                                );
                            }
                        }
                    }
                    RECORD_ORDER_FAILED => {
                        if let Some(mut rec) = decode_payload::<OrderFailedRecord>(payload) {
                            shard.release_frozen_for_order(
                                rec.user_id,
                                rec.order_id_hi,
                                rec.order_id_lo,
                            );
                            gw_stream.forward(&mut rec);
                        }
                    }
                    RECORD_CONFIG_APPLIED => {
                        if let Some(mut rec) = decode_payload::<ConfigAppliedRecord>(payload) {
                            shard.process_config_applied(rec.symbol_id, rec.config_version);
                            info!(
                                "config_applied: symbol={} v={}",
                                rec.symbol_id, rec.config_version,
                            );
                            gw_stream.forward(&mut rec);
                        }
                    }
                    _ => {}
                }
            });
            let Some((last_delivered_seq, gap)) = replay_range(&recv) else {
                continue;
            };
            // ME gap records carry fills and freeze lifecycle: they
            // are APPLIED to the shard and forwarded to the gateway
            // exactly like the live path. A discarded gap would leave
            // positions ≠ sum of fills permanently (invariant #4) —
            // the next live fill advances the tip past the gap, so
            // even a restart replay would never heal it.
            let new_tip = handle_replay(
                "risk.me",
                Some(&stream.replay_addr),
                stream.symbol_id,
                last_delivered_seq,
                gap,
                wal_dir,
                tls,
                |raw| apply_me_replay_record(shard, gw_stream, raw),
            );
            stream.receiver.reset_after_replay(new_tip);
        }

        // Orders/cancels from Gateway.
        loop {
            // Egress backpressure: if either output ring is
            // full a processed order's response/accepted push
            // would have nowhere to go. Stop draining the
            // socket this iteration — the kernel recv buffer
            // absorbs, the main loop drains the output rings
            // below, and we resume next iteration. Same end
            // iteration without dropping a message.
            if rings.response_producer.is_full() || rings.me_producer.is_full() {
                break;
            }
            let recv = gw_receiver.try_recv_with(|hdr, payload| {
                match hdr.record_type {
                    RECORD_ORDER_REQUEST => {
                        if let Some(order) = decode_payload::<OrderRequest>(payload) {
                            let risk_in_ns = time_ns();
                            // `risk_in` records an order arriving from
                            // gateway. t_us measured against the
                            // gateway's submit timestamp.
                            rsx_log::latency_sample!(
                                "risk_in",
                                order.order_id_hi,
                                order.order_id_lo,
                                order.timestamp_ns
                            );
                            // Process directly on this thread (no input
                            // ring). The loop head guarantees both output
                            // rings have a free slot, so neither push can
                            // fail — a dropped response/accepted would be
                            // a silent ghost order (gateway thinks it's
                            // pending, ME never sees it).
                            gauges.orders_processed.fetch_add(1, Ordering::Relaxed);
                            let resp = shard.process_order(&order);
                            if matches!(resp, OrderResponse::Accepted { .. }) {
                                rings
                                    .me_producer
                                    .push(MeCommand::Order(order, risk_in_ns))
                                    .expect(
                                        "INVARIANT: me_producer \
                                 capacity checked at loop head",
                                    );
                            }
                            rings.response_producer.push(resp).expect(
                                "INVARIANT: response_producer \
                             capacity checked at loop head",
                            );
                        }
                    }
                    RECORD_CANCEL_REQUEST => {
                        if let Some(cancel) = decode_payload::<CancelRequest>(payload) {
                            rings.me_producer.push(MeCommand::Cancel(cancel)).expect(
                                "INVARIANT: me_producer \
                                 capacity checked at loop head",
                            );
                        }
                    }
                    _ => {}
                }
            });
            if matches!(recv, CastRecvWith::Empty) {
                break;
            }
            let Some((last_delivered_seq, gap)) = replay_range(&recv) else {
                continue;
            };
            let new_tip = handle_replay(
                "risk.gw",
                cfg.gw_replay_addr.as_deref(),
                shard_id,
                last_delivered_seq,
                gap,
                wal_dir,
                tls,
                // Discard is safe by design for the GW order stream:
                // pre-ack orders lost in the gap are client-resent
                // and ME-deduped (rsx-matching documents this).
                // Draining is still required to clear FAULTED.
                |_| {},
            );
            gw_receiver.reset_after_replay(new_tip);
        }

        // Mark prices from Mark process
        loop {
            let recv = mark_receiver.try_recv_with(|preamble, payload| {
                if preamble.record_type == RECORD_MARK_PRICE {
                    if let Some(rec) = decode_payload::<MarkPriceRecord>(payload) {
                        // Latest-wins state; applied directly on the
                        // tile (no ring — recv and shard share a thread).
                        shard.update_mark(rec.symbol_id, rec.mark_price.0);
                    }
                }
            });
            if matches!(recv, CastRecvWith::Empty) {
                break;
            }
            let Some((last_delivered_seq, gap)) = replay_range(&recv) else {
                continue;
            };
            let new_tip = handle_replay(
                "risk.mark",
                cfg.mark_replay_addr.as_deref(),
                shard_id,
                last_delivered_seq,
                gap,
                wal_dir,
                tls,
                // Mark is latest-wins state: the next live mark
                // supersedes anything in the gap, so the drained
                // records need no application.
                |_| {},
            );
            mark_receiver.reset_after_replay(new_tip);
        }

        // Periodic risk work (liquidation sweep, tip persist,
        // stashed-BBO drain) — inputs are processed directly in
        // the recv handlers above.
        shard.tick(&mut rings, now_secs);

        // Publish load gauges (relaxed stores; once per loop
        // iteration, not per message — zero hot-path syscall cost).
        gauges
            .resp_ring_used
            .store(resp_cons.slots() as u64, Ordering::Relaxed);
        gauges
            .accept_ring_used
            .store(me_cons.slots() as u64, Ordering::Relaxed);
        gauges
            .persist_ring_used
            .store(shard.persist_ring_used(), Ordering::Relaxed);

        // Drain responses: ORDER_FAILED and liquidation notices to GW
        while let Ok(resp) = resp_cons.pop() {
            match resp {
                OrderResponse::Accepted { .. } => {}
                OrderResponse::Rejected {
                    user_id,
                    reason,
                    order_id_hi,
                    order_id_lo,
                } => {
                    gauges.rejects.fetch_add(1, Ordering::Relaxed);
                    let failure = FailureReason::from(reason);
                    let mut rec = order_failed_record(user_id, order_id_hi, order_id_lo, failure);
                    gw_stream.forward(&mut rec);
                }
                OrderResponse::Liquidation {
                    user_id,
                    symbol_id,
                    status,
                    round,
                    side,
                    qty,
                    price,
                    slip_bps,
                } => {
                    let mut rec = LiquidationRecord {
                        seq: 0,
                        ts_ns: time_ns(),
                        user_id,
                        symbol_id,
                        status,
                        side,
                        _pad0: [0; 2],
                        round,
                        qty,
                        price,
                        slip_bps,
                        _pad1: [0; 8],
                    };
                    gw_stream.forward(&mut rec);
                }
            }
        }

        // Drain ME commands -> cast to the ME that owns the symbol
        while let Ok(command) = me_cons.pop() {
            let (order, risk_in_ns) = match command {
                MeCommand::Order(order, risk_in_ns) => (order, risk_in_ns),
                MeCommand::Cancel(cancel) => {
                    send_cancel(me_senders, cancel);
                    continue;
                }
            };
            let send_failed = match me_senders.get_mut(&order.symbol_id) {
                Some(sender) => {
                    let mut msg = OrderMessage {
                        seq: 0,
                        price: order.price,
                        qty: order.qty,
                        side: order.side,
                        tif: order.tif,
                        reduce_only: if order.reduce_only { 1 } else { 0 },
                        post_only: if order.post_only { 1 } else { 0 },
                        _pad1: [0; 4],
                        user_id: order.user_id,
                        _pad2: 0,
                        timestamp_ns: order.timestamp_ns,
                        order_id_hi: order.order_id_hi,
                        order_id_lo: order.order_id_lo,
                        risk_in_ns,
                        _pad3: [0; 56],
                    };
                    match send_to_me(sender, &mut msg) {
                        Ok(()) => false,
                        Err(e) => {
                            warn!("risk: forward order to me failed: {e}");
                            true
                        }
                    }
                }
                None => {
                    warn!("order for unknown symbol_id={}", order.symbol_id);
                    true
                }
            };
            // The pre-trade gate already froze margin in-memory for
            // this accepted order. If ME never receives it, no
            // RECORD_ORDER_ACCEPTED returns → the freeze (and the
            // gateway's pending order) would leak forever. Release
            // the in-memory freeze and tell the client it failed.
            // (The durable PG freeze is only written by confirm_freeze
            // on OrderAccepted, so there is nothing durable to undo.)
            if send_failed {
                shard.release_frozen_for_order(order.user_id, order.order_id_hi, order.order_id_lo);
                let mut rec = order_failed_record(
                    order.user_id,
                    order.order_id_hi,
                    order.order_id_lo,
                    FailureReason::NetworkError,
                );
                gw_stream.forward(&mut rec);
            }
        }

        // Cast housekeeping
        for sender in me_senders.values_mut() {
            if let Err(e) = sender.tick() {
                warn!("risk: me_sender tick failed: {e}");
            }
            sender.recv_control();
        }
        gw_stream.tick(clock);

        // Persist-worker death (circuit open after sustained flush
        // failure, persist.rs CIRCUIT_AT) must not leave a Live
        // shard trading with no PG write-behind while the lease
        // keeps renewing. Treat it like lease loss: the caller
        // tears down and re-enters warm catchup — releasing the
        // advisory lock lets a standby take over.
        if persist_handle.is_finished() {
            return LiveExit::PersistWorkerDied;
        }

        // Check lease health (non-blocking — lease thread updates
        // atomically). On loss the caller tears down the persist
        // worker + lease thread (each owns a PG connection) and
        // re-enters warm catchup — this process becomes a standby.
        if !lease_held.load(Ordering::Relaxed) {
            if lease_error.load(Ordering::Relaxed) {
                return LiveExit::LeaseError;
            }
            return LiveExit::LeaseLost;
        }
    }
}

/// `(last_delivered_seq, gap)` of a sticky FAULTED/RECONNECT
/// receive, for `handle_replay`; `None` while the receiver is
/// healthy.
fn replay_range(recv: &CastRecvWith) -> Option<(u64, Option<(u64, u64)>)> {
    match *recv {
        CastRecvWith::Data | CastRecvWith::Empty => None,
        CastRecvWith::Faulted {
            last_delivered_seq,
            gap_start,
            gap_end_inclusive,
        } => Some((last_delivered_seq, Some((gap_start, gap_end_inclusive)))),
        CastRecvWith::Reconnect { last_delivered_seq } => Some((last_delivered_seq, None)),
    }
}

/// Forward a cancel to its symbol's ME.
fn send_cancel(me_senders: &mut HashMap<u32, CastSender>, mut cancel: CancelRequest) {
    let Some(sender) = me_senders.get_mut(&cancel.symbol_id) else {
        warn!("cancel for unknown symbol_id={}", cancel.symbol_id);
        return;
    };
    if let Err(e) = send_to_me(sender, &mut cancel) {
        warn!("risk: forward cancel to me failed: {e}");
    }
}

/// A risk-originated reject or send failure for the gateway
/// stream. `seq` is 0 here; `GwStream::forward` stamps it.
fn order_failed_record(
    user_id: u32,
    order_id_hi: u64,
    order_id_lo: u64,
    reason: FailureReason,
) -> OrderFailedRecord {
    OrderFailedRecord {
        seq: 0,
        ts_ns: time_ns(),
        user_id,
        _pad0: 0,
        order_id_hi,
        order_id_lo,
        reason: reason as u8,
        _pad: [0; 23],
    }
}
