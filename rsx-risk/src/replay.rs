use crate::account::Account;
use crate::insurance::InsuranceFund;
use crate::position::Position;
use crate::shard::RiskShard;
use crate::types::FillEvent;
use rsx_cast::ReplicationConsumer;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::wal::extract_seq;
use rustc_hash::FxHashMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::info;
use tracing::warn;

/// Drain a replication-replay stream after `CastRecv::Faulted`
/// or `CastRecv::Reconnect`. Mirrors
/// `rsx_matching::replay::drain_dxs_replay_into_book` but
/// delegates record application to a caller-supplied closure,
/// so the same drain helper serves all four risk receivers
/// (gw / me / mark / replica-tip), each with its own
/// local-state apply.
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
        let r = rt.block_on(consumer.run_once(
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
        attempts += 1;
        match &r {
            Err(e) if e.kind() == io::ErrorKind::NotFound
                && attempts < MAX_RETRIES =>
            {
                warn!(
                    "risk replay not available (attempt \
                     {attempts}), retrying in 15ms"
                );
                std::thread::sleep(
                    std::time::Duration::from_millis(15),
                );
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

pub struct ColdStartState {
    pub accounts: FxHashMap<u32, Account>,
    pub positions: FxHashMap<(u32, u32), Position>,
    pub tips: Vec<u64>,
    pub insurance_funds: FxHashMap<u32, InsuranceFund>,
    /// Per-order frozen margin reservations, keyed by
    /// `(order_id_hi as u128) << 64 | order_id_lo as u128`.
    /// Source of truth — aggregate is derived.
    pub frozen_orders: FxHashMap<u128, (u32, i64)>,
}

pub async fn load_from_postgres(
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
            &[
                &(shard_count as i32),
                &(shard_id as i32),
            ],
        )
        .await?;
    for row in &rows {
        let user_id: i32 = row.get(0);
        let mut acct = Account::new(
            user_id as u32,
            row.get::<_, i64>(1),
        );
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
            &[
                &(shard_count as i32),
                &(shard_id as i32),
            ],
        )
        .await?;
    for row in &rows {
        let uid: i32 = row.get(0);
        let sid: i32 = row.get(1);
        let mut pos =
            Position::new(uid as u32, sid as u32);
        pos.long_qty = row.get::<_, i64>(2);
        pos.short_qty = row.get::<_, i64>(3);
        pos.long_entry_cost = row.get::<_, i64>(4);
        pos.short_entry_cost = row.get::<_, i64>(5);
        pos.realized_pnl = row.get::<_, i64>(6);
        pos.last_fill_seq =
            row.get::<_, i64>(7) as u64;
        pos.version = row.get::<_, i64>(8) as u64;
        positions
            .insert((uid as u32, sid as u32), pos);
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
        let mut fund =
            InsuranceFund::new(sid as u32, row.get(1));
        fund.version = row.get::<_, i64>(2) as u64;
        insurance_funds.insert(sid as u32, fund);
    }

    let mut frozen_orders = FxHashMap::default();
    let rows = client
        .query(
            "SELECT user_id, order_id_hi, order_id_lo, \
             amount FROM frozen_orders \
             WHERE user_id % $1 = $2",
            &[
                &(shard_count as i32),
                &(shard_id as i32),
            ],
        )
        .await?;
    for row in &rows {
        let uid: i32 = row.get(0);
        let hi: i64 = row.get(1);
        let lo: i64 = row.get(2);
        let amount: i64 = row.get(3);
        let key = ((hi as u64 as u128) << 64)
            | (lo as u64 as u128);
        frozen_orders.insert(key, (uid as u32, amount));
    }

    Ok(ColdStartState {
        accounts,
        positions,
        tips,
        insurance_funds,
        frozen_orders,
    })
}

pub fn replay_from_wal(
    shard: &mut RiskShard,
    wal_dir: &Path,
    symbol_ids: &[u32],
) -> std::io::Result<u64> {
    use rsx_cast::decode_payload;
    use rsx_messages::decode_fill_record;
    use rsx_messages::OrderAcceptedRecord;
    use rsx_messages::OrderCancelledRecord;
    use rsx_messages::OrderDoneRecord;
    use rsx_messages::OrderFailedRecord;
    use rsx_cast::WalReader;
    use rsx_messages::RECORD_ORDER_ACCEPTED;
    use rsx_messages::RECORD_ORDER_FAILED;
    use rsx_messages::RECORD_ORDER_CANCELLED;
    use rsx_messages::RECORD_ORDER_DONE;
    use rsx_messages::RECORD_FILL;

    let mut replayed = 0u64;
    for &sid in symbol_ids {
        assert!(
            (sid as usize) < shard.tips.len(),
            "symbol_id {} exceeds tips len {}",
            sid,
            shard.tips.len(),
        );
        let tip = shard.tips[sid as usize];
        let start_seq = tip + 1;
        let mut reader = WalReader::open_from_seq(
            sid, start_seq, wal_dir,
        )?;
        while let Some(raw) = reader.next()? {
            match raw.header.record_type {
                RECORD_FILL => {
                    let fill = match decode_fill_record(
                        &raw.payload,
                    ) {
                        Some(f) => f,
                        None => continue,
                    };
                    shard.process_fill(&FillEvent {
                        seq: fill.seq,
                        symbol_id: fill.symbol_id,
                        taker_user_id: fill.taker_user_id,
                        maker_user_id: fill.maker_user_id,
                        price: fill.price.0,
                        qty: fill.qty.0,
                        taker_side: fill.taker_side,
                        timestamp_ns: fill.ts_ns,
                    });
                    replayed += 1;
                }
                RECORD_ORDER_DONE => {
                    if let Some(rec) = decode_payload::<OrderDoneRecord>(&raw.payload) {
                        shard.release_frozen_for_order(
                            rec.user_id,
                            rec.order_id_hi,
                            rec.order_id_lo,
                        );
                    }
                }
                RECORD_ORDER_CANCELLED => {
                    if let Some(rec) = decode_payload::<OrderCancelledRecord>(&raw.payload) {
                        shard.release_frozen_for_order(
                            rec.user_id,
                            rec.order_id_hi,
                            rec.order_id_lo,
                        );
                    }
                }
                RECORD_ORDER_FAILED => {
                    if let Some(rec) = decode_payload::<OrderFailedRecord>(&raw.payload) {
                        shard.release_frozen_for_order(
                            rec.user_id,
                            rec.order_id_hi,
                            rec.order_id_lo,
                        );
                    }
                }
                RECORD_ORDER_ACCEPTED => {
                    if let Some(rec) = decode_payload::<OrderAcceptedRecord>(&raw.payload) {
                        if shard.user_in_shard(rec.user_id)
                            && rec.reduce_only == 0
                        {
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
                _ => {}
            }
        }
    }
    Ok(replayed)
}

pub async fn acquire_advisory_lock(
    client: &Client,
    shard_id: u32,
) -> Result<(), Error> {
    client
        .execute(
            "SELECT pg_advisory_lock($1::bigint)",
            &[&(shard_id as i64)],
        )
        .await?;
    Ok(())
}
