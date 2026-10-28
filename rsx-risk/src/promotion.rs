//! Warm-standby promotion: every node catches up from the ME
//! replication stream, then polls the non-blocking advisory lock.
//! The lock — never catch-up — is the single-main fence
//! (invariant #10).

use crate::lease::AdvisoryLease;
use crate::replay::apply_record;
use crate::shard::RiskShard;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::CaughtUpRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::TlsConfig;
use rsx_cast::RECORD_CAUGHT_UP;
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
/// path. NO cast socket, NO persist worker, NO gateway
/// ingress/egress, NO liquidation tick — this node is a passive
/// follower.
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
/// return the per-symbol applied ME seqs (indexed by `symbol_id`)
/// — the caller transitions to LIVE with the warm shard (no
/// rebuild) and resumes each ME receiver after its seq. The
/// advisory lock is the SOLE single-main fence; catch-up only
/// gates WHEN `try_acquire` is called.
///
#[allow(clippy::too_many_arguments)]
pub fn run_warm_catchup(
    rt: &tokio::runtime::Runtime,
    pg_client: &tokio_postgres::Client,
    lease: &mut AdvisoryLease,
    shard: &mut RiskShard,
    me_streams: &[(u32, String)],
    wal_dir: &str,
    lease_poll_interval_ms: u64,
    tls: &TlsConfig,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
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
            return Ok(applied_tips(shard, &streams));
        }

        // Stream ME records until CAUGHT_UP (callback returns
        // false) or the connection ends. `caught_live_seq` is set
        // by the callback when it sees RECORD_CAUGHT_UP.
        let mut all_caught_up = true;
        for stream in &mut streams {
            let drain = match drain_round(rt, stream, shard) {
                Ok(drain) => drain,
                Err(e) => {
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
            };
            all_caught_up &= drain == Drain::CaughtUp;
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
        // starts with no gap. A running ME keeps advancing, so
        // each stream streams to CAUGHT_UP until it is level.
        for stream in &mut streams {
            drain_until_caught_up(poll, || drain_round(rt, stream, shard))
                .map_err(|e| format!("ME stream {} final drain: {e}", stream.symbol_id))?;
        }
        return Ok(applied_tips(shard, &streams));
    }
}

struct WarmStream {
    symbol_id: u32,
    consumer: ReplicationConsumer,
    applied_seq: u64,
}

/// The shard's fill tips with each warm stream's applied seq (any
/// record type) laid over its symbol, indexed by `symbol_id`.
fn applied_tips(shard: &RiskShard, streams: &[WarmStream]) -> Vec<u64> {
    let mut tips = shard.tips.clone();
    for stream in streams {
        if let Some(tip) = tips.get_mut(stream.symbol_id as usize) {
            *tip = stream.applied_seq;
        }
    }
    tips
}

const FINAL_DRAIN_ROUNDS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drain {
    CaughtUp,
    Behind,
    NoMarker,
}

fn classify_drain(live_seq: Option<u64>, applied_seq: u64) -> Drain {
    match live_seq {
        None => Drain::NoMarker,
        Some(tip) if applied_seq >= tip => Drain::CaughtUp,
        Some(_) => Drain::Behind,
    }
}

fn drain_round(
    rt: &tokio::runtime::Runtime,
    stream: &mut WarmStream,
    shard: &mut RiskShard,
) -> std::io::Result<Drain> {
    let live_seq = stream_to_caught_up(rt, stream, shard)?;
    Ok(classify_drain(live_seq, stream.applied_seq))
}

/// Run `round` until it reports `CaughtUp`, sleeping `poll` after
/// each `Behind`. A stream that ends without a marker, or is still
/// behind after `FINAL_DRAIN_ROUNDS`, is an error.
fn drain_until_caught_up(
    poll: Duration,
    mut round: impl FnMut() -> std::io::Result<Drain>,
) -> std::io::Result<()> {
    for _ in 0..FINAL_DRAIN_ROUNDS {
        match round()? {
            Drain::CaughtUp => return Ok(()),
            Drain::NoMarker => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "stream ended without CAUGHT_UP",
                ));
            }
            Drain::Behind => std::thread::sleep(poll),
        }
    }
    Err(std::io::Error::other(format!(
        "still behind after {FINAL_DRAIN_ROUNDS} rounds"
    )))
}

/// Stream one ME's records into the shard until the server's
/// CAUGHT_UP marker. `Ok(Some(live_seq))` is the marker's live seq
/// (0 for a brand-new ME with nothing at or after tip 0, which is
/// caught up by definition); `Ok(None)` means the stream ended
/// without a marker. A round that streamed no record reports the
/// applied seq: the server then echoes the requested `tip + 1` as
/// `live_seq`, a seq the ME does not have. A nonzero tip the ME
/// cannot serve is an error: missing history must block promotion.
fn stream_to_caught_up(
    rt: &tokio::runtime::Runtime,
    stream: &mut WarmStream,
    shard: &mut RiskShard,
) -> std::io::Result<Option<u64>> {
    let mut caught_live_seq: Option<u64> = None;
    let mut streamed = false;
    let result = rt.block_on(stream.consumer.run_once(|raw| {
        if raw.header.record_type == RECORD_CAUGHT_UP {
            caught_live_seq = decode_payload::<CaughtUpRecord>(&raw.payload).map(|record| {
                if streamed {
                    record.live_seq
                } else {
                    stream.applied_seq
                }
            });
            return false;
        }
        streamed = true;
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        stream.applied_seq = stream.applied_seq.max(seq);
        apply_record(shard, raw.header.record_type, &raw.payload);
        true
    }));
    match result {
        Ok(()) => Ok(caught_live_seq),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && stream.consumer.tip == 0 => {
            info!(
                "warm catchup: ME has nothing at/after tip+1 \
                 (tip={}, empty ME or behind us) — caught up",
                stream.consumer.tip,
            );
            Ok(Some(0))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[path = "promotion_test.rs"]
mod promotion_test;
