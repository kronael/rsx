use crate::account::Account;
use crate::funding::load_funding_settled;
use crate::insurance::InsuranceFund;
use crate::margin::Freeze;
use crate::position::Position;
use crate::rings::GwStream;
use crate::shard::order_key;
use crate::shard::RiskShard;
use crate::types::BboUpdate;
use crate::types::FillEvent;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::CastRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::TlsConfig;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_messages::decode_fill_record;
use rsx_messages::BboRecord;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rustc_hash::FxHashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::info;
use tracing::warn;

/// Drain a replication-replay stream after `CastRecv::Faulted`
/// or `CastRecv::Reconnect`. Mirrors
/// `rsx_matching::replay::drain_dxs_replay_into_book` but
/// delegates record application to a caller-supplied closure,
/// so the same drain helper serves all three risk receivers
/// (gw / me / mark), each with its own local-state apply.
///
/// Builds a single-threaded tokio runtime on demand — FAULTED
/// is rare enough that the few-millisecond setup cost is
/// negligible. Returns the highest seq applied (`new_tip`).
/// The caller is expected to invoke
/// `CastReceiver::reset_after_replay(new_tip)` to clear the
/// sticky FAULTED/RECONNECT state and resume live UDP.
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
    // Pre-seed tip so the request starts at last_delivered + 1
    // regardless of stale on-disk tip.
    consumer.tip = last_delivered_seq;

    let mut new_tip = last_delivered_seq;
    let mut applied = 0u64;
    let mut skipped = 0u64;
    // Retry on NotFound: WAL flushes every 10ms. Missing seq may
    // be buffered in memory. Three retries at 15ms covers the window.
    const MAX_RETRIES: u8 = 3;
    let mut attempts = 0u8;
    let result = loop {
        let r = rt.block_on(consumer.run_once(|raw: RawWalRecord| -> bool {
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
        attempts += 1;
        match &r {
            Err(e) if e.kind() == io::ErrorKind::NotFound && attempts < MAX_RETRIES => {
                warn!(
                    "risk replay not available (attempt \
                     {attempts}), retrying in 15ms"
                );
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            _ => break r,
        }
    };
    if let Err(e) = result {
        warn!(
            "risk replay stream ended with error: {e} \
             (applied={applied} skipped={skipped} \
             new_tip={new_tip})",
        );
        return Err(e);
    }
    info!(
        "risk replay drained: stream_id={stream_id} \
         applied={applied} skipped={skipped} new_tip={new_tip}",
    );
    Ok(new_tip)
}

/// Handle a `CastRecv::Faulted` or `CastRecv::Reconnect` by
/// draining the producer's replication stream from
/// `last_delivered_seq + 1`, feeding every gap record through
/// `apply`. State application is the caller's closure: ME streams
/// pass `apply_me_replay_record` (a discarded ME gap leaves
/// positions ≠ sum of fills permanently, invariant #4); the gw and
/// mark call sites document why a discard is safe for them, and
/// may pass `replay_addr = None` to skip the gap outright.
/// Returns the new tip to pass into
/// `CastReceiver::reset_after_replay`.
///
/// Panics if the replication consumer exhausts its retry budget.
/// Transient connection errors retry inside `drain_replay`.
#[allow(clippy::too_many_arguments)]
pub fn handle_replay<F: FnMut(&RawWalRecord)>(
    label: &str,
    replay_addr: Option<&str>,
    stream_id: u32,
    last_delivered_seq: u64,
    gap: Option<(u64, u64)>,
    wal_dir: &str,
    tls: &TlsConfig,
    mut apply: F,
) -> u64 {
    let Some(replay_addr) = replay_addr else {
        let skip_to = gap.map(|(_, ge)| ge).unwrap_or(last_delivered_seq);
        warn!(
            "{label} gap at seq={last_delivered_seq} with no replay \
             endpoint configured; skipping to seq={skip_to}"
        );
        return skip_to;
    };
    match gap {
        Some((gs, ge)) => warn!(
            "{label} FAULTED at seq={last_delivered_seq} \
             gap=[{gs}..={ge}], opening replay via {replay_addr}",
        ),
        None => warn!(
            "{label} RECONNECT at seq={last_delivered_seq}, \
             opening replay via {replay_addr}",
        ),
    }
    let tip_file =
        std::path::PathBuf::from(wal_dir).join(format!("risk_{label}_{stream_id}_replay_tip.bin"));
    // Retry if the WAL hasn't flushed the gap records yet.
    // WAL flushes every 10ms; 5 retries × 15ms = 75ms covers
    // the window plus some slack for burst writes.
    let gap_end = gap.map(|(_, ge)| ge).unwrap_or(0);
    const MAX_TIP_RETRIES: u8 = 5;
    let mut tip_retries = 0u8;
    // Resume each retry from the previous attempt's tip so an
    // already-applied (and already-forwarded) record is never fed
    // through `apply` twice.
    let mut resume_from = last_delivered_seq;
    let new_tip = loop {
        let tip = drain_replay(
            stream_id,
            replay_addr.to_owned(),
            resume_from,
            tip_file.clone(),
            tls.clone(),
            &mut apply,
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
            resume_from = resume_from.max(tip);
            std::thread::sleep(Duration::from_millis(15));
        } else {
            break tip;
        }
    };
    info!("{label} replay drained, new_tip={new_tip}, resuming",);
    new_tip
}

/// Apply one replayed ME record during a live FAULTED/RECONNECT
/// gap drain: shard state goes through the same `apply_record`
/// path warm catchup uses, then the record is forwarded to the
/// gateway for exactly the record types the live recv loop
/// forwards. Liquidation halt/resume pacing and latency samples
/// are live-loop side effects, not state, and are skipped — a
/// stale halt inside the gap self-corrects on the next live
/// ORDER_FAILED or fill.
pub fn apply_me_replay_record(shard: &mut RiskShard, gw: &mut GwStream, raw: &RawWalRecord) {
    let record_type = raw.header.record_type;
    apply_record(shard, record_type, &raw.payload);
    match record_type {
        RECORD_FILL => forward_payload::<FillRecord>(gw, &raw.payload),
        RECORD_ORDER_DONE => forward_payload::<OrderDoneRecord>(gw, &raw.payload),
        RECORD_ORDER_INSERTED => forward_payload::<OrderInsertedRecord>(gw, &raw.payload),
        RECORD_ORDER_FAILED => forward_payload::<OrderFailedRecord>(gw, &raw.payload),
        RECORD_CONFIG_APPLIED => forward_payload::<ConfigAppliedRecord>(gw, &raw.payload),
        _ => {}
    }
}

fn forward_payload<T: CastRecord>(gw: &mut GwStream, payload: &[u8]) {
    if let Some(mut record) = decode_payload::<T>(payload) {
        gw.forward(&mut record);
    }
}

pub struct ColdStartState {
    pub accounts: FxHashMap<u32, Account>,
    pub positions: FxHashMap<(u32, u32), Position>,
    pub tips: Vec<u64>,
    pub insurance_funds: FxHashMap<u32, InsuranceFund>,
    /// Per-order frozen margin reservations, keyed by
    /// `(order_id_hi as u128) << 64 | order_id_lo as u128`.
    /// Source of truth — aggregate is derived.
    pub frozen_orders: FxHashMap<u128, Freeze>,
    /// Last settled funding interval per symbol, indexed by symbol id.
    pub funding_settled: Vec<Option<u64>>,
}

/// Load this shard's state in one repeatable-read snapshot, so the
/// collateral and the settled funding intervals it reflects agree.
pub async fn load_from_postgres(
    client: &Client,
    shard_id: u32,
    shard_count: u32,
    max_symbols: usize,
) -> Result<ColdStartState, Error> {
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await?;
    let state = read_state(client, shard_id, shard_count, max_symbols).await;
    let end = if state.is_ok() { "COMMIT" } else { "ROLLBACK" };
    client.batch_execute(end).await?;
    state
}

async fn read_state(
    client: &Client,
    shard_id: u32,
    shard_count: u32,
    max_symbols: usize,
) -> Result<ColdStartState, Error> {
    let mut accounts = FxHashMap::default();
    let rows = client
        .query(
            "SELECT user_id, collateral, version \
             FROM accounts \
             WHERE user_id % $1 = $2",
            &[&(shard_count as i32), &(shard_id as i32)],
        )
        .await?;
    for row in &rows {
        let user_id: i32 = row.get(0);
        let mut acct = Account::new(user_id as u32, row.get::<_, i64>(1));
        acct.version = row.get::<_, i64>(2) as u64;
        accounts.insert(user_id as u32, acct);
    }

    let mut positions = FxHashMap::default();
    let rows = client
        .query(
            "SELECT user_id, symbol_id, long_qty, \
             short_qty, long_entry_cost, \
             short_entry_cost, realized_pnl, \
             last_fill_seq, version \
             FROM positions \
             WHERE user_id % $1 = $2",
            &[&(shard_count as i32), &(shard_id as i32)],
        )
        .await?;
    for row in &rows {
        let uid: i32 = row.get(0);
        let sid: i32 = row.get(1);
        let mut pos = Position::new(uid as u32, sid as u32);
        pos.long_qty = row.get::<_, i64>(2);
        pos.short_qty = row.get::<_, i64>(3);
        pos.long_entry_cost = row.get::<_, i64>(4);
        pos.short_entry_cost = row.get::<_, i64>(5);
        pos.realized_pnl = row.get::<_, i64>(6);
        pos.last_fill_seq = row.get::<_, i64>(7) as u64;
        pos.version = row.get::<_, i64>(8) as u64;
        positions.insert((uid as u32, sid as u32), pos);
    }

    let mut tips = vec![0u64; max_symbols];
    let rows = client
        .query(
            "SELECT symbol_id, last_seq FROM tips \
             WHERE instance_id = $1",
            &[&(shard_id as i32)],
        )
        .await?;
    for row in &rows {
        let sid: i32 = row.get(0);
        let seq: i64 = row.get(1);
        if (sid as usize) < max_symbols {
            tips[sid as usize] = seq as u64;
        }
    }

    let mut insurance_funds = FxHashMap::default();
    let rows = client
        .query(
            "SELECT symbol_id, balance, version \
             FROM insurance_fund",
            &[],
        )
        .await?;
    for row in &rows {
        let sid: i32 = row.get(0);
        let mut fund = InsuranceFund::new(sid as u32, row.get(1));
        fund.version = row.get::<_, i64>(2) as u64;
        insurance_funds.insert(sid as u32, fund);
    }

    let mut frozen_orders = FxHashMap::default();
    let rows = client
        .query(
            "SELECT user_id, order_id_hi, order_id_lo, \
             symbol_id, amount FROM frozen_orders \
             WHERE user_id % $1 = $2",
            &[&(shard_count as i32), &(shard_id as i32)],
        )
        .await?;
    for row in &rows {
        let uid: i32 = row.get(0);
        let hi: i64 = row.get(1);
        let lo: i64 = row.get(2);
        let sid: i32 = row.get(3);
        let amount: i64 = row.get(4);
        let key = order_key(hi as u64, lo as u64);
        frozen_orders.insert(
            key,
            Freeze {
                user_id: uid as u32,
                symbol_id: sid as u32,
                amount,
            },
        );
    }

    let funding_settled = load_funding_settled(client, shard_id, max_symbols).await?;

    Ok(ColdStartState {
        accounts,
        positions,
        tips,
        insurance_funds,
        frozen_orders,
        funding_settled,
    })
}

/// Apply one ME WAL record to the shard. Shared by warm catch-up
/// and the live FAULTED/RECONNECT gap drain, both fed from the ME
/// replication stream. Returns `true` if the record was a FILL (so
/// callers can count fills replayed).
///
/// This is the authoritative state-apply path: it mutates shard
/// position/freeze/mark/index/config state only — no egress, no
/// liquidation halt/resume, no latency traces. Those live in the LIVE
/// loop because they are forward-to-gateway side effects, not state.
/// Idempotent re-apply is safe: `process_fill` dedups on the
/// per-symbol tip (invariant #5), and freeze insert/release is keyed by
/// order_id.
pub fn apply_record(shard: &mut RiskShard, record_type: u16, payload: &[u8]) -> bool {
    match record_type {
        RECORD_FILL => {
            if let Some(fill) = decode_fill_record(payload) {
                shard.process_fill(&FillEvent::from(&fill));
                return true;
            }
        }
        RECORD_ORDER_DONE => {
            if let Some(rec) = decode_payload::<OrderDoneRecord>(payload) {
                shard.release_frozen_for_order(rec.user_id, rec.order_id_hi, rec.order_id_lo);
            }
        }
        RECORD_ORDER_FAILED => {
            if let Some(rec) = decode_payload::<OrderFailedRecord>(payload) {
                shard.release_frozen_for_order(rec.user_id, rec.order_id_hi, rec.order_id_lo);
            }
        }
        RECORD_ORDER_ACCEPTED => {
            if let Some(rec) = decode_payload::<OrderAcceptedRecord>(payload) {
                if shard.user_in_shard(rec.user_id) && rec.reduce_only == 0 {
                    shard.replay_freeze_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                        rec.price,
                        rec.qty,
                        rec.symbol_id,
                    );
                }
            }
        }
        RECORD_BBO => {
            if let Some(rec) = decode_payload::<BboRecord>(payload) {
                shard.process_bbo(&BboUpdate::from(&rec));
            }
        }
        RECORD_CONFIG_APPLIED => {
            if let Some(rec) = decode_payload::<ConfigAppliedRecord>(payload) {
                shard.process_config_applied(rec.symbol_id, rec.config_version);
            }
        }
        _ => {}
    }
    false
}
