//! Failover / warm-standby lifecycle for the risk process.
//!
//! Holds the non-hot-loop machinery the risk `main` drives around
//! its live order loop: FAULTED/RECONNECT replay drain
//! (`handle_replay`), gateway-stream forwarding (`forward_to_gw`),
//! and the warm-catchup promotion protocol (`run_warm_catchup` +
//! `drain_mark_warm`). `main.rs` keeps the entrypoint, the restart
//! loop, and the live receive loop.

use crate::lease::AdvisoryLease;
use crate::replay::apply_record;
use crate::shard::RiskShard;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::CaughtUpRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::TlsConfig;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_messages::MarkPriceRecord;
use rsx_messages::RECORD_MARK_PRICE;
use std::env;
use std::time::Duration;
use tracing::info;
use tracing::warn;

/// Handle a `CastRecv::Faulted` or `CastRecv::Reconnect` by
/// draining the producer's replication stream from
/// `last_delivered_seq + 1`. Returns the new tip to pass into
/// `CastReceiver::reset_after_replay`.
///
/// The apply path is intentionally minimal for round 1 — each
/// record is just acknowledged so the receiver can resume.
/// Re-injecting orders/fills/marks into the live ring is a
/// follow-up (see .ship/28-REFINE-AUDIT-2/PLAN.md). The spec
/// invariant "ME never silently drops" is preserved because
/// matching has its own FAULTED handler that re-runs the
/// authoritative replay from risk's WAL.
///
/// Panics if the replay endpoint env var is unset (fail-loud
/// per the spec) or the replication consumer exhausts its
/// retry budget. Transient connection errors retry inside
/// `drain_replay`.
pub fn handle_replay(
    label: &str,
    env_var: &str,
    stream_id: u32,
    last_delivered_seq: u64,
    gap: Option<(u64, u64)>,
    wal_dir: &str,
    tls: &TlsConfig,
) -> u64 {
    match gap {
        Some((gs, ge)) => warn!(
            "{label} FAULTED at seq={last_delivered_seq} \
             gap=[{gs}..={ge}], opening replay via {env_var}",
        ),
        None => warn!(
            "{label} RECONNECT at seq={last_delivered_seq}, \
             opening replay via {env_var}",
        ),
    }
    let replay_addr = env::var(env_var).unwrap_or_else(|_| {
        panic!(
            "{label} {} requires {env_var} pointing at the \
             producer's replication server",
            if gap.is_some() { "FAULTED" } else { "RECONNECT" },
        )
    });
    let tip_file = std::path::PathBuf::from(wal_dir).join(
        format!("risk_{label}_{stream_id}_replay_tip.bin"),
    );
    // Retry if the WAL hasn't flushed the gap records yet.
    // WAL flushes every 10ms; 5 retries × 15ms = 75ms covers
    // the window plus some slack for burst writes.
    let gap_end = gap.map(|(_, ge)| ge).unwrap_or(0);
    const MAX_TIP_RETRIES: u8 = 5;
    let mut tip_retries = 0u8;
    let new_tip = loop {
        let tip = crate::drain_replay(
            stream_id,
            replay_addr.clone(),
            last_delivered_seq,
            tip_file.clone(),
            tls.clone(),
            |raw| {
                let seq = rsx_cast::wal::extract_seq(
                    &raw.payload,
                ).unwrap_or(0);
                tracing::debug!(
                    "{label} replay applied \
                     record_type={} seq={}",
                    raw.header.record_type, seq,
                );
            },
        )
        .unwrap_or_else(|e| {
            panic!(
                "{label} replay drain failed against \
                 {replay_addr}: {e}",
            )
        });
        tip_retries += 1;
        if tip < gap_end && tip_retries < MAX_TIP_RETRIES {
            warn!(
                "{label} replay tip={tip} < gap_end={gap_end}, \
                 WAL not flushed yet (attempt {tip_retries}), \
                 retrying in 15ms"
            );
            std::thread::sleep(
                Duration::from_millis(15),
            );
        } else {
            break tip;
        }
    };
    info!(
        "{label} replay drained, new_tip={new_tip}, resuming",
    );
    new_tip
}

/// Forward a record onto risk's gateway stream, renumbering it
/// with `gw`'s own contiguous seq (SEQ-1 fix). Risk's gateway
/// stream multiplexes forwarded ME records AND risk-generated
/// margin rejects; preserving ME's seq (or the reject's seq=0)
/// leaves holes the gateway reads as FAULTED, and seq=0 records
/// are dropped outright by the receiver. The gateway never
/// replays *from* risk, so renumbering is safe — the seq is
/// transport-only on this hop; the record is identified by its
/// order_id. CRC is recomputed by `send_raw` over the restamped
/// payload.
pub fn forward_to_gw(
    gw: &mut CastSender,
    record_type: u16,
    payload: &[u8],
) {
    let plen = payload.len();
    if !(8..=256).contains(&plen) {
        warn!("risk: gw forward bad payload len={plen}");
        return;
    }
    let mut buf = [0u8; 256];
    buf[..plen].copy_from_slice(payload);
    let seq = gw.next_seq();
    buf[0..8].copy_from_slice(&seq.to_le_bytes());
    if let Err(e) = gw.send_raw(record_type, &buf[..plen]) {
        warn!("risk: forward to gw failed: {e}");
    }
    gw.advance_seq();
}

/// WARM CATCHUP (NodeState::WarmCatchup).
///
/// Consume the live main's authoritative ME WAL replication
/// stream (the SAME source `handle_replay` uses for FAULTED
/// recovery — no separate risk WAL) into the already-PG-loaded
/// shard, applying each record via the shared `apply_record`
/// path. Also drain the mark stream into `update_mark`. NO
/// persist worker, NO gateway ingress/egress, NO liquidation
/// tick — this node is a passive follower.
///
/// CAUGHT-UP detection: the replication server emits
/// RECORD_CAUGHT_UP { live_seq } after draining its current WAL.
/// `caught_up` ⟺ we have seen that record AND `applied_seq >=
/// live_seq`. We open the consumer with a per-node tip file so a
/// reconnect resumes from the persisted tip+1 (CAUGHT_UP itself
/// carries no seq, so it never advances the tip).
///
/// PROMOTE: only when caught up do we call the NON-BLOCKING
/// `pg_try_advisory_lock`. If it fails another node holds the
/// lock — stay warm and retry after `lease_poll_interval_ms`. If
/// it succeeds this node is the sole holder (invariant #10); we
/// do a FINAL DRAIN of any records past the last CAUGHT_UP and
/// return Ok — the caller transitions to LIVE with the warm
/// shard (no rebuild). The advisory lock is the SOLE single-main
/// fence; catch-up only gates WHEN `try_acquire` is called.
///
/// ME topology: the live main binds ONE CastReceiver for all MEs
/// and replays a single stream_id, so this is ONE
/// ReplicationConsumer — matching that topology.
#[allow(clippy::too_many_arguments)]
pub fn run_warm_catchup(
    rt: &tokio::runtime::Runtime,
    pg_client: &tokio_postgres::Client,
    lease: &mut AdvisoryLease,
    shard: &mut RiskShard,
    mark_receiver: &mut CastReceiver,
    me_stream_id: u32,
    me_repl_addr: &str,
    wal_dir: &str,
    lease_poll_interval_ms: u64,
    tls: &TlsConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let tip_file = std::path::PathBuf::from(wal_dir).join(
        format!("risk_warm_me_{me_stream_id}_tip.bin"),
    );
    let mut consumer = ReplicationConsumer::new(
        me_stream_id,
        vec![me_repl_addr.to_owned()],
        tip_file,
        tls.clone(),
    )?;
    // Resume the ME stream from the shard's persisted per-symbol
    // tip so we don't re-request records already folded into the
    // PG snapshot (process_fill still dedups on tip — invariant
    // #5 — but skipping the re-request is cheaper).
    if (me_stream_id as usize) < shard.tips.len() {
        consumer.tip = shard.tips[me_stream_id as usize];
    }

    info!(
        "warm catchup: consuming ME replication \
         stream_id={me_stream_id} from {me_repl_addr} tip={}",
        consumer.tip,
    );

    let mut applied_seq: u64 = consumer.tip;
    let poll = Duration::from_millis(lease_poll_interval_ms.max(1));

    loop {
        // Drain mark prices (latest-wins state) each iteration so
        // the warm shard's margin view stays fresh; mark gaps are
        // recoverable (latest-wins) so we ignore FAULTED here.
        drain_mark_warm(mark_receiver, shard);

        // Stream ME records until CAUGHT_UP (callback returns
        // false) or the connection ends. `caught_live_seq` is set
        // by the callback when it sees RECORD_CAUGHT_UP.
        let mut caught_live_seq: Option<u64> = None;
        let stream = rt.block_on(consumer.run_once(|raw| {
            if raw.header.record_type == RECORD_CAUGHT_UP {
                if let Some(rec) =
                    decode_payload::<CaughtUpRecord>(&raw.payload)
                {
                    caught_live_seq = Some(rec.live_seq);
                }
                // Stop the stream so the outer loop can poll the
                // lock; reconnect resumes from tip+1.
                return false;
            }
            let seq = extract_seq(&raw.payload).unwrap_or(0);
            if seq > applied_seq {
                applied_seq = seq;
            }
            apply_record(
                shard,
                raw.header.record_type,
                &raw.payload,
            );
            true
        }));

        if let Err(e) = stream {
            // RECORD_REPLICATION_NOT_AVAILABLE maps to NotFound: ME
            // cannot serve a replay from our tip+1. Two cases, both
            // "nothing to catch up": (1) tip==0 fresh cluster — ME's
            // WAL is empty (my_highest=0), there is nothing to apply;
            // (2) tip>0 — ME restarted empty / GC'd its tail, and our
            // PG snapshot already covers up to consumer.tip, so we are
            // ahead of ME. Either way, caught up → proceed to the lock.
            if e.kind() == std::io::ErrorKind::NotFound {
                info!(
                    "warm catchup: ME has nothing at/after tip+1 \
                     (tip={}, empty ME or behind us) — caught up",
                    consumer.tip,
                );
                caught_live_seq = Some(0);
                applied_seq = consumer.tip;
            } else {
                // Disconnect/error clears caught_up implicitly (we
                // re-derive it next iteration). Back off then retry;
                // the consumer reconnects from its persisted tip+1.
                warn!(
                    "warm catchup: ME stream error: {e}; \
                     retry in {}ms",
                    poll.as_millis(),
                );
                std::thread::sleep(poll);
                continue;
            }
        }

        let caught_up = match caught_live_seq {
            Some(live_seq) => applied_seq >= live_seq,
            None => false,
        };

        if !caught_up {
            // Connection ended without CAUGHT_UP, or we are
            // behind the reported live_seq. Loop to re-stream
            // (resumes from tip+1) — no lock attempt.
            continue;
        }

        // Caught up: attempt the NON-BLOCKING lock. This is the
        // ONLY place try_acquire is called; the lock — not
        // catch-up — is the single-main fence (invariant #10).
        let acquired = rt
            .block_on(lease.try_acquire(pg_client))?;
        if !acquired {
            // Another node is main. Stay warm; keep applying.
            std::thread::sleep(poll);
            continue;
        }

        info!(
            "warm catchup: caught up (applied_seq={applied_seq}) \
             AND won advisory lock — final drain then go LIVE",
        );

        // FINAL DRAIN: between the last CAUGHT_UP and winning the
        // lock the main may have written more records. Apply
        // everything up to the current WAL tip so the live loop
        // starts with no gap. One more run_once: stream to the
        // next CAUGHT_UP and stop.
        let final_drain = rt.block_on(consumer.run_once(|raw| {
            if raw.header.record_type == RECORD_CAUGHT_UP {
                return false;
            }
            let seq = extract_seq(&raw.payload).unwrap_or(0);
            if seq > applied_seq {
                applied_seq = seq;
            }
            apply_record(
                shard,
                raw.header.record_type,
                &raw.payload,
            );
            true
        }));
        if let Err(e) = final_drain {
            warn!(
                "warm catchup: final drain stream error: {e} \
                 (applied_seq={applied_seq}); proceeding — the \
                 live ME receiver re-syncs via FAULTED replay",
            );
        }
        drain_mark_warm(mark_receiver, shard);
        return Ok(());
    }
}

/// Drain the mark CastReceiver into the shard during warm
/// catchup. Mark is latest-wins state; FAULTED/RECONNECT are
/// ignored (the next live mark supersedes any gap).
pub fn drain_mark_warm(
    mark_receiver: &mut CastReceiver,
    shard: &mut RiskShard,
) {
    loop {
        let recv = mark_receiver.try_recv_with(|preamble, payload| {
            if preamble.record_type == RECORD_MARK_PRICE {
                if let Some(rec) =
                    decode_payload::<MarkPriceRecord>(payload)
                {
                    shard.update_mark(
                        rec.symbol_id,
                        rec.mark_price.0,
                    );
                }
            }
        });
        match recv {
            CastRecvWith::Data => {}
            _ => break,
        }
    }
}
