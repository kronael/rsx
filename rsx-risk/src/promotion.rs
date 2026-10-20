//! Warm-standby promotion: every node catches up from the ME
//! replication stream, then polls the non-blocking advisory lock.
//! The lock — never catch-up — is the single-main fence
//! (invariant #10).

use crate::lease::AdvisoryLease;
use crate::replay::apply_record;
use crate::shard::RiskShard;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::CaughtUpRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::TlsConfig;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_messages::MarkPriceRecord;
use rsx_messages::RECORD_MARK_PRICE;
use std::time::Duration;
use tracing::info;
use tracing::warn;

/// Node role in the eager warm-standby protocol. Every process
/// boots into `WarmCatchup`, consumes the main's authoritative
/// ME replication stream, and only transitions to `Live` once it
/// is caught up AND wins the (non-blocking) advisory lock. There
/// is no separate "cold main boot" — promotion is always warm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    /// Applying the ME replication stream into the shard with no
    /// persist worker, no gateway ingress, no egress, no
    /// liquidation tick. Polling the non-blocking lock once
    /// caught up.
    WarmCatchup,
    /// Sole lock holder (invariant #10). Persist worker, lease
    /// renewal, gateway ingress + egress, liquidation tick all
    /// attached. Applying ME records AND forwarding to GW.
    Live,
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
#[allow(clippy::too_many_arguments)]
pub fn run_warm_catchup(
    rt: &tokio::runtime::Runtime,
    pg_client: &tokio_postgres::Client,
    lease: &mut AdvisoryLease,
    shard: &mut RiskShard,
    mark_receiver: &mut CastReceiver,
    me_streams: &[(u32, String)],
    wal_dir: &str,
    lease_poll_interval_ms: u64,
    tls: &TlsConfig,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    struct WarmStream {
        symbol_id: u32,
        consumer: ReplicationConsumer,
        applied_seq: u64,
    }
    let mut streams = Vec::with_capacity(me_streams.len());
    for (symbol_id, replay_addr) in me_streams {
        let tip_file =
            std::path::PathBuf::from(wal_dir).join(format!("risk_warm_me_{symbol_id}_tip.bin"));
        let mut consumer = ReplicationConsumer::new(
            *symbol_id,
            vec![replay_addr.to_owned()],
            tip_file,
            tls.clone(),
        )?;
        if (*symbol_id as usize) < shard.tips.len() {
            consumer.tip = shard.tips[*symbol_id as usize];
        }
        let applied_seq = consumer.tip;
        info!("warm catchup: symbol={symbol_id} from {replay_addr} tip={applied_seq}");
        streams.push(WarmStream {
            symbol_id: *symbol_id,
            consumer,
            applied_seq,
        });
    }
    let poll = Duration::from_millis(lease_poll_interval_ms.max(1));

    loop {
        // A standby can sit here indefinitely waiting for the lock,
        // so it has to honour SIGTERM like any other state -- else
        // ops can only stop it with SIGKILL.
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            info!("shutdown requested during warm catchup");
            return Ok(());
        }

        // Drain mark prices (latest-wins state) each iteration so
        // the warm shard's margin view stays fresh; mark gaps are
        // recoverable (latest-wins) so we ignore FAULTED here.
        drain_mark_warm(mark_receiver, shard);

        // Stream ME records until CAUGHT_UP (callback returns
        // false) or the connection ends. `caught_live_seq` is set
        // by the callback when it sees RECORD_CAUGHT_UP.
        let mut all_caught_up = true;
        for stream in &mut streams {
            let mut caught_live_seq: Option<u64> = None;
            let result = rt.block_on(stream.consumer.run_once(|raw| {
                if raw.header.record_type == RECORD_CAUGHT_UP {
                    caught_live_seq = decode_payload::<CaughtUpRecord>(&raw.payload)
                        .map(|record| record.live_seq);
                    return false;
                }
                let seq = extract_seq(&raw.payload).unwrap_or(0);
                stream.applied_seq = stream.applied_seq.max(seq);
                apply_record(shard, raw.header.record_type, &raw.payload);
                true
            }));
            if let Err(e) = result {
                // RECORD_REPLICATION_NOT_AVAILABLE maps to NotFound: ME
                // cannot serve a replay from our tip+1. Only a brand-new
                // stream at tip zero is safely empty. A nonzero tip with
                // missing history stays uncaught and blocks promotion.
                if e.kind() == std::io::ErrorKind::NotFound && stream.consumer.tip == 0 {
                    info!(
                        "warm catchup: ME has nothing at/after tip+1 \
                     (tip={}, empty ME or behind us) — caught up",
                        stream.consumer.tip,
                    );
                    caught_live_seq = Some(0);
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
                    all_caught_up = false;
                    break;
                }
            }
            all_caught_up &= caught_live_seq.is_some_and(|live_seq| stream.applied_seq >= live_seq);
        }
        if !all_caught_up {
            // Connection ended without CAUGHT_UP, or we are
            // behind the reported live_seq. Loop to re-stream
            // (resumes from tip+1) — no lock attempt.
            continue;
        }

        // Caught up: attempt the NON-BLOCKING lock. This is the
        // ONLY place try_acquire is called; the lock — not
        // catch-up — is the single-main fence (invariant #10).
        let acquired = rt.block_on(lease.try_acquire(pg_client))?;
        if !acquired {
            // Another node is main. Stay warm; keep applying.
            std::thread::sleep(poll);
            continue;
        }

        info!("warm catchup: all ME streams caught up and advisory lock acquired",);

        // FINAL DRAIN: between the last CAUGHT_UP and winning the
        // lock the main may have written more records. Apply
        // everything up to the current WAL tip so the live loop
        // starts with no gap. One more run_once: stream to the
        // next CAUGHT_UP and stop.
        for stream in &mut streams {
            let mut final_live_seq = None;
            let result = rt.block_on(stream.consumer.run_once(|raw| {
                if raw.header.record_type == RECORD_CAUGHT_UP {
                    final_live_seq = decode_payload::<CaughtUpRecord>(&raw.payload)
                        .map(|record| record.live_seq);
                    return false;
                }
                let seq = extract_seq(&raw.payload).unwrap_or(0);
                stream.applied_seq = stream.applied_seq.max(seq);
                apply_record(shard, raw.header.record_type, &raw.payload);
                true
            }));
            if let Err(error) = result {
                if error.kind() == std::io::ErrorKind::NotFound && stream.consumer.tip == 0 {
                    final_live_seq = Some(0);
                } else {
                    return Err(error.into());
                }
            }
            if final_live_seq.is_none_or(|tip| stream.applied_seq < tip) {
                return Err(
                    format!("ME stream {} advanced during promotion", stream.symbol_id).into(),
                );
            }
        }
        drain_mark_warm(mark_receiver, shard);
        return Ok(());
    }
}

/// Drain the mark CastReceiver into the shard during warm
/// catchup. Mark is latest-wins state; FAULTED/RECONNECT are
/// ignored (the next live mark supersedes any gap).
pub fn drain_mark_warm(mark_receiver: &mut CastReceiver, shard: &mut RiskShard) {
    loop {
        let recv = mark_receiver.try_recv_with(|preamble, payload| {
            if preamble.record_type == RECORD_MARK_PRICE {
                if let Some(rec) = decode_payload::<MarkPriceRecord>(payload) {
                    shard.update_mark(rec.symbol_id, rec.mark_price.0);
                }
            }
        });
        match recv {
            CastRecvWith::Data => {}
            _ => break,
        }
    }
}
