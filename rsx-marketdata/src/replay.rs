use rsx_dxs::CaughtUpRecord;
use rsx_dxs::DxsConsumer;
use rsx_dxs::FillRecord;
use rsx_dxs::OrderCancelledRecord;
use rsx_dxs::OrderInsertedRecord;
use rsx_dxs::RawWalRecord;
use rsx_dxs::RECORD_CAUGHT_UP;
use rsx_dxs::RECORD_FILL;
use rsx_dxs::RECORD_ORDER_CANCELLED;
use rsx_dxs::RECORD_ORDER_INSERTED;
use std::path::PathBuf;
use tracing::info;

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

pub fn run_replay_bootstrap_blocking(
    stream_id: u32,
    replay_addr: String,
    tip_file: PathBuf,
) -> std::io::Result<ReplayResult> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_replay_bootstrap(
        stream_id,
        replay_addr,
        tip_file,
    ))
}

async fn run_replay_bootstrap(
    stream_id: u32,
    replay_addr: String,
    tip_file: PathBuf,
) -> std::io::Result<ReplayResult> {
    let mut consumer = DxsConsumer::new(
        stream_id,
        replay_addr,
        tip_file,
        None,
    )?;

    let events = std::sync::Arc::new(
        std::sync::Mutex::new(Vec::new()),
    );
    let caught_up = std::sync::Arc::new(
        std::sync::Mutex::new(false),
    );
    let last_seq = std::sync::Arc::new(
        std::sync::Mutex::new(0u64),
    );

    let events_clone = events.clone();
    let caught_up_clone = caught_up.clone();
    let last_seq_clone = last_seq.clone();

    consumer
        .run(move |record: RawWalRecord| {
            let mut evs = events_clone.lock().unwrap();
            let mut cu = caught_up_clone.lock().unwrap();
            let mut ls = last_seq_clone.lock().unwrap();

            match record.header.record_type {
                RECORD_ORDER_INSERTED => {
                    if record.payload.len()
                        >= std::mem::size_of::<OrderInsertedRecord>()
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                record.payload.as_ptr()
                                    as *const OrderInsertedRecord,
                            )
                        };
                        evs.push(ReplayEvent {
                            record_type: RECORD_ORDER_INSERTED,
                            insert: Some(rec),
                            cancel: None,
                            fill: None,
                        });
                        *ls = rec.seq;
                    }
                }
                RECORD_ORDER_CANCELLED => {
                    if record.payload.len()
                        >= std::mem::size_of::<OrderCancelledRecord>()
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                record.payload.as_ptr()
                                    as *const OrderCancelledRecord,
                            )
                        };
                        evs.push(ReplayEvent {
                            record_type: RECORD_ORDER_CANCELLED,
                            insert: None,
                            cancel: Some(rec),
                            fill: None,
                        });
                        *ls = rec.seq;
                    }
                }
                RECORD_FILL => {
                    if record.payload.len()
                        >= std::mem::size_of::<FillRecord>()
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                record.payload.as_ptr()
                                    as *const FillRecord,
                            )
                        };
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
                    if record.payload.len()
                        >= std::mem::size_of::<CaughtUpRecord>()
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                record.payload.as_ptr()
                                    as *const CaughtUpRecord,
                            )
                        };
                        *cu = true;
                        *ls = rec.live_seq;
                        info!(
                            "replay caught up at seq={}",
                            rec.live_seq
                        );
                    }
                }
                _ => {}
            }
        })
        .await?;

    let events = std::sync::Arc::try_unwrap(events)
        .unwrap()
        .into_inner()
        .unwrap();
    let caught_up = *caught_up.lock().unwrap();
    let last_seq = *last_seq.lock().unwrap();

    Ok(ReplayResult {
        events,
        caught_up,
        last_seq,
    })
}

