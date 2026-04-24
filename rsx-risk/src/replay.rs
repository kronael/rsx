use crate::account::Account;
use crate::insurance::InsuranceFund;
use crate::position::Position;
use crate::shard::RiskShard;
use crate::types::FillEvent;
use rustc_hash::FxHashMap;
use std::path::Path;
use tokio_postgres::Client;
use tokio_postgres::Error;

pub struct ColdStartState {
    pub accounts: FxHashMap<u32, Account>,
    pub positions: FxHashMap<(u32, u32), Position>,
    pub tips: Vec<u64>,
    pub insurance_funds: FxHashMap<u32, InsuranceFund>,
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
            "SELECT user_id, collateral, \
             frozen_margin, version \
             FROM accounts \
             WHERE user_id % $1 = $2",
            &[
                &(shard_count as i32),
                &(shard_id as i32),
            ],
        )
        .await?;
    // If RSX_RISK_RESET_FROZEN_ON_START=1, zero the
    // persisted frozen_margin on cold start. WAL replay
    // will re-freeze any still-pending orders (ACCEPTED
    // since tip+1). Older pending orders lose their
    // reservation — acceptable in dev to escape stale
    // leaks accumulated across runs. NEVER set true in
    // production without a reconciliation against the ME
    // book state.
    let reset_frozen = std::env::var(
        "RSX_RISK_RESET_FROZEN_ON_START",
    )
    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    .unwrap_or(false);
    for row in &rows {
        let user_id: i32 = row.get(0);
        let mut acct = Account::new(
            user_id as u32,
            row.get::<_, i64>(1),
        );
        acct.frozen_margin = if reset_frozen {
            0
        } else {
            row.get::<_, i64>(2)
        };
        acct.version = row.get::<_, i64>(3) as u64;
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

    Ok(ColdStartState {
        accounts,
        positions,
        tips,
        insurance_funds,
    })
}

pub fn replay_from_wal(
    shard: &mut RiskShard,
    wal_dir: &Path,
    symbol_ids: &[u32],
) -> std::io::Result<u64> {
    use rsx_dxs::decode_fill_record;
    use rsx_dxs::records::OrderAcceptedRecord;
    use rsx_dxs::records::OrderCancelledRecord;
    use rsx_dxs::records::OrderDoneRecord;
    use rsx_dxs::records::OrderFailedRecord;
    use rsx_dxs::WalReader;
    use rsx_dxs::RECORD_ORDER_ACCEPTED;
    use rsx_dxs::RECORD_ORDER_FAILED;
    use rsx_dxs::RECORD_ORDER_CANCELLED;
    use rsx_dxs::RECORD_ORDER_DONE;
    use rsx_dxs::RECORD_FILL;

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
                RECORD_ORDER_DONE
                    if raw.payload.len()
                        >= std::mem::size_of::<
                            OrderDoneRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            raw.payload.as_ptr()
                                as *const OrderDoneRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                }
                RECORD_ORDER_CANCELLED
                    if raw.payload.len()
                        >= std::mem::size_of::<
                            OrderCancelledRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            raw.payload.as_ptr()
                                as *const
                                    OrderCancelledRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                }
                RECORD_ORDER_FAILED
                    if raw.payload.len()
                        >= std::mem::size_of::<
                            OrderFailedRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            raw.payload.as_ptr()
                                as *const
                                    OrderFailedRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                }
                RECORD_ORDER_ACCEPTED
                    if raw.payload.len()
                        >= std::mem::size_of::<
                            OrderAcceptedRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            raw.payload.as_ptr()
                                as *const
                                    OrderAcceptedRecord,
                        )
                    };
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
