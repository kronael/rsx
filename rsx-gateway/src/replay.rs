//! `drain_replay`: FAULTED/RECONNECT recovery for the
//! gateway's cast receiver. Mirrors `rsx_matching::replay`
//! and `rsx_risk::replay::drain_replay`; the apply callback
//! is supplied by the caller so the gateway can decide how
//! to route any records that arrive during replay (today:
//! re-emit to clients via route_*; round-1 leaves this as a
//! debug log — see `main.rs::handle_replay`).

use rsx_cast::wal::extract_seq;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::TlsConfig;
use rsx_cast::RECORD_CAUGHT_UP;
use std::io;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;

/// Drain a replication-replay stream after
/// `CastRecv::Faulted` / `CastRecv::Reconnect`. Returns the
/// highest seq applied. The caller must invoke
/// `CastReceiver::reset_after_replay(new_tip)` to clear the
/// sticky state and resume live UDP delivery.
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
            "gateway replay stream ended with error: {e} \
             (applied={applied} skipped={skipped} \
             new_tip={new_tip})",
        );
        return Err(e);
    }
    info!(
        "gateway replay drained: applied={applied} \
         skipped={skipped} new_tip={new_tip}",
    );
    Ok(new_tip)
}
