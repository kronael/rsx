use rsx_cast::CaughtUpRecord;
use rsx_cast::decode_payload;
use rsx_cast::ReplicationConsumer;
use rsx_messages::FillRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_cast::RawWalRecord;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_cast::wal::extract_seq;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_INSERTED;
use std::io;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;

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
    mut apply: F,
) -> io::Result<u64>
where
    F: FnMut(&RawWalRecord),
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![replay_addr],
        tip_file,
        None,
    )?;
    consumer.tip = last_delivered_seq;

    let mut new_tip = last_delivered_seq;
    let mut applied = 0u64;
    let mut skipped = 0u64;
    let result = rt.block_on(consumer.run_once(
        |raw: RawWalRecord| -> bool {
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
        },
    ));
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

#[derive(Debug)]
pub struct ReplayEvent {
    pub record_type: u16,
    pub insert: Option<OrderInsertedRecord>,
    pub cancel: Option<OrderCancelledRecord>,
    pub fill: Option<FillRecord>,
}

pub struct ReplayResult {
    pub events: Vec<ReplayEvent>,
    pub caught_up: bool,
    pub last_seq: u64,
}

/// Blocking wrapper. Creates a single-threaded tokio
/// runtime, runs replay, returns when caught up.
pub fn run_replay_bootstrap_blocking(
    stream_id: u32,
    replay_addr: String,
    tip_file: PathBuf,
) -> std::io::Result<ReplayResult> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_replay_bootstrap(
        stream_id,
        replay_addr,
        tip_file,
    ))
}

/// Async replay: connect once, consume records until
/// CaughtUp, then return.
pub async fn run_replay_bootstrap(
    stream_id: u32,
    replay_addr: String,
    tip_file: PathBuf,
) -> std::io::Result<ReplayResult> {
    let mut consumer = ReplicationConsumer::new(
        stream_id,
        vec![replay_addr],
        tip_file,
        None,
    )?;

    let mut events = Vec::new();
    let mut caught_up = false;
    let mut last_seq = 0u64;

    // run_once callback borrows local state via raw
    // pointers. Safe because run_once is synchronous
    // w.r.t. the callback — it calls it inline on the
    // same task, never across threads.
    let ev_ptr = &mut events as *mut Vec<ReplayEvent>;
    let cu_ptr = &mut caught_up as *mut bool;
    let ls_ptr = &mut last_seq as *mut u64;

    consumer
        .run_once(move |record: RawWalRecord| {
            // SAFETY: callback runs inline on same
            // thread/task as the caller. Pointers are
            // valid for the duration of run_once.
            let evs = unsafe { &mut *ev_ptr };
            let cu = unsafe { &mut *cu_ptr };
            let ls = unsafe { &mut *ls_ptr };

            match record.header.record_type {
                RECORD_ORDER_INSERTED => {
                    if let Some(rec) = decode_payload::<OrderInsertedRecord>(&record.payload) {
                        evs.push(ReplayEvent {
                            record_type:
                                RECORD_ORDER_INSERTED,
                            insert: Some(rec),
                            cancel: None,
                            fill: None,
                        });
                        *ls = rec.seq;
                    }
                }
                RECORD_ORDER_CANCELLED => {
                    if let Some(rec) = decode_payload::<OrderCancelledRecord>(&record.payload) {
                        evs.push(ReplayEvent {
                            record_type:
                                RECORD_ORDER_CANCELLED,
                            insert: None,
                            cancel: Some(rec),
                            fill: None,
                        });
                        *ls = rec.seq;
                    }
                }
                RECORD_FILL => {
                    if let Some(rec) = decode_payload::<FillRecord>(&record.payload) {
                        evs.push(ReplayEvent {
                            record_type: RECORD_FILL,
                            insert: None,
                            cancel: None,
                            fill: Some(rec),
                        });
                        *ls = rec.seq;
                    }
                }
                RECORD_CAUGHT_UP => {
                    if let Some(rec) = decode_payload::<CaughtUpRecord>(&record.payload) {
                        *cu = true;
                        *ls = rec.live_seq;
                        info!(
                            "replay caught up at \
                             seq={}",
                            rec.live_seq
                        );
                    }
                    return false;
                }
                _ => {}
            }
            true
        })
        .await?;

    Ok(ReplayResult {
        events,
        caught_up,
        last_seq,
    })
}
