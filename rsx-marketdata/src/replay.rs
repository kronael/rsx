use rsx_cast::wal::extract_seq;
use rsx_cast::RawWalRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::TlsConfig;
use rsx_cast::RECORD_CAUGHT_UP;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tracing::info;
use tracing::warn;

const STALL_TIMEOUT: Duration = Duration::from_secs(1);

/// Drain a replication-replay stream after
/// `CastRecv::Faulted` / `CastRecv::Reconnect` on a live ME
/// receiver. Mirrors `rsx_matching::replay` /
/// `rsx_risk::replay::drain_replay` / `rsx_gateway::replay`.
/// Returns the new tip — caller is expected to call
/// `CastReceiver::reset_after_replay(new_tip)`.
pub fn drain_replay<F>(
    stream_id: u32,
    replay_addr: String,
    last_delivered_seq: u64,
    tip_file: PathBuf,
    tls: TlsConfig,
    mut apply: F,
) -> io::Result<u64>
where
    F: FnMut(&RawWalRecord),
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut consumer = ReplicationConsumer::new(stream_id, vec![replay_addr], tip_file, tls)?;
    consumer.tip = last_delivered_seq;

    let mut new_tip = last_delivered_seq;
    let mut applied = 0u64;
    let mut skipped = 0u64;
    let result = rt.block_on(consumer.run_once(|raw: RawWalRecord| -> bool {
        if raw.header.record_type == RECORD_CAUGHT_UP {
            return false;
        }
        let seq = extract_seq(&raw.payload).unwrap_or(0);
        if seq <= last_delivered_seq {
            skipped += 1;
            return true;
        }
        if seq > new_tip {
            new_tip = seq;
        }
        apply(&raw);
        applied += 1;
        true
    }));
    if let Err(e) = result {
        warn!(
            "marketdata replay stream ended with error: {e} \
             (applied={applied} skipped={skipped} \
             new_tip={new_tip})",
        );
        return Err(e);
    }
    info!(
        "marketdata replay drained: applied={applied} \
         skipped={skipped} new_tip={new_tip}",
    );
    Ok(new_tip)
}

pub struct ReplayResult {
    pub events: Vec<RawWalRecord>,
    pub caught_up: bool,
    pub last_seq: u64,
}

/// Blocking wrapper. Creates a single-threaded tokio
/// runtime, runs replay, returns when caught up.
pub fn run_replay_bootstrap_blocking(
    stream_id: u32,
    replay_addr: String,
    tip_file: PathBuf,
    tls: TlsConfig,
) -> std::io::Result<ReplayResult> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_replay_bootstrap(stream_id, replay_addr, tip_file, tls))
}

/// Async replay: collect every record until CaughtUp, then return. The
/// caller folds the records with `apply::apply_record`. The server leaves
/// the tail of a long reply in its TLS send buffer until its next write, so
/// a round that goes quiet is dropped and the rest is requested from the tip.
pub async fn run_replay_bootstrap(
    stream_id: u32,
    replay_addr: String,
    tip_file: PathBuf,
    tls: TlsConfig,
) -> std::io::Result<ReplayResult> {
    let mut consumer = ReplicationConsumer::new(stream_id, vec![replay_addr], tip_file, tls)?;

    let mut events = Vec::new();
    let mut caught_up = false;

    loop {
        let round = consumer.run_once(|record: RawWalRecord| {
            if record.header.record_type == RECORD_CAUGHT_UP {
                caught_up = true;
                return false;
            }
            events.push(record);
            true
        });
        match tokio::time::timeout(STALL_TIMEOUT, round).await {
            Ok(result) => break result?,
            Err(_) => info!(
                "replay stalled at seq={}, requesting the rest",
                consumer.tip
            ),
        }
    }

    info!("replay caught up={caught_up} at seq={}", consumer.tip);
    Ok(ReplayResult {
        events,
        caught_up,
        last_seq: consumer.tip,
    })
}
