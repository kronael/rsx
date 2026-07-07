use crate::account::Account;
use crate::insurance::InsuranceFund;
use crate::position::Position;
use rtrb::Consumer;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::error;
use tracing::info;
use tracing::warn;

#[derive(Clone, Debug)]
pub enum PersistEvent {
    Position(Position),
    Account(Account),
    Fill(FillRecord),
    Tip {
        symbol_id: u32,
        seq: u64,
    },
    Funding(FundingRecord),
    InsuranceFund(InsuranceFund),
    Liquidation(LiquidationRecord),
    FrozenInsert(FrozenOrderRecord),
    FrozenRemove {
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
    },
}

#[derive(Clone, Debug)]
pub struct FrozenOrderRecord {
    pub user_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub symbol_id: u32,
    pub amount: i64,
}

#[derive(Clone, Debug)]
pub struct FillRecord {
    pub symbol_id: u32,
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub price: i64,
    pub qty: i64,
    pub taker_fee: i64,
    pub maker_fee: i64,
    pub taker_side: u8,
    pub seq: u64,
    pub timestamp_ns: u64,
}

#[derive(Clone, Debug)]
pub struct FundingRecord {
    pub user_id: u32,
    pub symbol_id: u32,
    pub amount: i64,
    pub rate: i64,
    pub settlement_ts: u64,
}

#[derive(Clone, Debug)]
pub struct LiquidationRecord {
    pub user_id: u32,
    pub symbol_id: u32,
    pub round: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
    pub slippage_bps: i64,
    pub status: u8,
    pub timestamp_ns: u64,
}

pub async fn upsert_positions(
    tx: &tokio_postgres::Transaction<'_>,
    positions: &[Position],
) -> Result<(), Error> {
    for p in positions {
        tx.execute(
            "INSERT INTO positions \
             (user_id, symbol_id, long_qty, short_qty, \
              long_entry_cost, short_entry_cost, \
              realized_pnl, last_fill_seq, version) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
             ON CONFLICT (user_id, symbol_id) \
             DO UPDATE SET \
               long_qty = $3, short_qty = $4, \
               long_entry_cost = $5, \
               short_entry_cost = $6, \
               realized_pnl = $7, \
               last_fill_seq = $8, version = $9",
            &[
                &(p.user_id as i32),
                &(p.symbol_id as i32),
                &p.long_qty,
                &p.short_qty,
                &p.long_entry_cost,
                &p.short_entry_cost,
                &p.realized_pnl,
                &(p.last_fill_seq as i64),
                &(p.version as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn upsert_accounts(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[Account],
) -> Result<(), Error> {
    for a in accounts {
        tx.execute(
            "INSERT INTO accounts \
             (user_id, collateral, version) \
             VALUES ($1,$2,$3) \
             ON CONFLICT (user_id) \
             DO UPDATE SET \
               collateral = $2, version = $3",
            &[&(a.user_id as i32), &a.collateral, &(a.version as i64)],
        )
        .await?;
    }
    Ok(())
}

pub async fn upsert_frozen_orders(
    tx: &tokio_postgres::Transaction<'_>,
    frozen: &[FrozenOrderRecord],
) -> Result<(), Error> {
    for f in frozen {
        tx.execute(
            "INSERT INTO frozen_orders \
             (user_id, order_id_hi, order_id_lo, \
              symbol_id, amount) \
             VALUES ($1,$2,$3,$4,$5) \
             ON CONFLICT (user_id, order_id_hi, order_id_lo) \
             DO UPDATE SET \
               symbol_id = $4, amount = $5",
            &[
                &(f.user_id as i32),
                &(f.order_id_hi as i64),
                &(f.order_id_lo as i64),
                &(f.symbol_id as i32),
                &f.amount,
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn delete_frozen_orders(
    tx: &tokio_postgres::Transaction<'_>,
    keys: &[(u32, u64, u64)],
) -> Result<(), Error> {
    for &(user_id, hi, lo) in keys {
        tx.execute(
            "DELETE FROM frozen_orders \
             WHERE user_id = $1 \
               AND order_id_hi = $2 \
               AND order_id_lo = $3",
            &[&(user_id as i32), &(hi as i64), &(lo as i64)],
        )
        .await?;
    }
    Ok(())
}

pub async fn insert_fills(
    tx: &tokio_postgres::Transaction<'_>,
    fills: &[FillRecord],
) -> Result<(), Error> {
    for f in fills {
        tx.execute(
            "INSERT INTO fills \
             (symbol_id, taker_user_id, maker_user_id, \
              price, qty, taker_fee, maker_fee, \
              taker_side, seq, timestamp_ns) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[
                &(f.symbol_id as i32),
                &(f.taker_user_id as i32),
                &(f.maker_user_id as i32),
                &f.price,
                &f.qty,
                &f.taker_fee,
                &f.maker_fee,
                &(f.taker_side as i16),
                &(f.seq as i64),
                &(f.timestamp_ns as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn upsert_tips(
    tx: &tokio_postgres::Transaction<'_>,
    instance_id: u32,
    tips: &[(u32, u64)],
) -> Result<(), Error> {
    for &(symbol_id, seq) in tips {
        tx.execute(
            "INSERT INTO tips \
             (instance_id, symbol_id, last_seq) \
             VALUES ($1,$2,$3) \
             ON CONFLICT (instance_id, symbol_id) \
             DO UPDATE SET last_seq = $3",
            &[&(instance_id as i32), &(symbol_id as i32), &(seq as i64)],
        )
        .await?;
    }
    Ok(())
}

pub async fn insert_funding(
    tx: &tokio_postgres::Transaction<'_>,
    payments: &[FundingRecord],
) -> Result<(), Error> {
    for p in payments {
        tx.execute(
            "INSERT INTO funding \
             (user_id, symbol_id, amount, rate, \
              settlement_ts) \
             VALUES ($1,$2,$3,$4,$5)",
            &[
                &(p.user_id as i32),
                &(p.symbol_id as i32),
                &p.amount,
                &p.rate,
                &(p.settlement_ts as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn upsert_insurance_funds(
    tx: &tokio_postgres::Transaction<'_>,
    funds: &[InsuranceFund],
) -> Result<(), Error> {
    for f in funds {
        tx.execute(
            "INSERT INTO insurance_fund \
             (symbol_id, balance, version) \
             VALUES ($1,$2,$3) \
             ON CONFLICT (symbol_id) \
             DO UPDATE SET balance = $2, version = $3",
            &[&(f.symbol_id as i32), &f.balance, &(f.version as i64)],
        )
        .await?;
    }
    Ok(())
}

pub async fn insert_liquidations(
    tx: &tokio_postgres::Transaction<'_>,
    events: &[LiquidationRecord],
) -> Result<(), Error> {
    for e in events {
        tx.execute(
            "INSERT INTO liquidations \
             (user_id, symbol_id, round, side, price, \
              qty, slippage_bps, status, timestamp_ns) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[
                &(e.user_id as i32),
                &(e.symbol_id as i32),
                &(e.round as i32),
                &(e.side as i16),
                &e.price,
                &e.qty,
                &(e.slippage_bps as i32),
                &(e.status as i16),
                &(e.timestamp_ns as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn flush_batch(
    client: &mut Client,
    shard_id: u32,
    events: &[PersistEvent],
) -> Result<(), Error> {
    if events.is_empty() {
        return Ok(());
    }

    let mut positions = Vec::new();
    let mut accounts = Vec::new();
    let mut fills = Vec::new();
    let mut tips = Vec::new();
    let mut funding = Vec::new();
    let mut insurance_funds = Vec::new();
    let mut liquidations = Vec::new();
    let mut frozen_inserts = Vec::new();
    let mut frozen_removes = Vec::new();
    for e in events {
        match e {
            PersistEvent::Position(p) => positions.push(p.clone()),
            PersistEvent::Account(a) => accounts.push(a.clone()),
            PersistEvent::Fill(f) => fills.push(f.clone()),
            PersistEvent::Tip { symbol_id, seq } => tips.push((*symbol_id, *seq)),
            PersistEvent::Funding(fp) => funding.push(fp.clone()),
            PersistEvent::InsuranceFund(fund) => insurance_funds.push(fund.clone()),
            PersistEvent::Liquidation(liq) => liquidations.push(liq.clone()),
            PersistEvent::FrozenInsert(f) => frozen_inserts.push(f.clone()),
            PersistEvent::FrozenRemove {
                user_id,
                order_id_hi,
                order_id_lo,
            } => frozen_removes.push((*user_id, *order_id_hi, *order_id_lo)),
        }
    }

    let tx = client.transaction().await?;
    upsert_positions(&tx, &positions).await?;
    upsert_accounts(&tx, &accounts).await?;
    upsert_frozen_orders(&tx, &frozen_inserts).await?;
    delete_frozen_orders(&tx, &frozen_removes).await?;
    insert_fills(&tx, &fills).await?;
    upsert_tips(&tx, shard_id, &tips).await?;
    insert_funding(&tx, &funding).await?;
    upsert_insurance_funds(&tx, &insurance_funds).await?;
    insert_liquidations(&tx, &liquidations).await?;
    tx.commit().await?;
    Ok(())
}

/// Flush interval (normal path).
const FLUSH_INTERVAL_MS: u64 = 10;
/// Initial backoff on flush error (ms).
const BACKOFF_INIT_MS: u64 = 100;
/// Maximum backoff between retries (ms).
const BACKOFF_MAX_MS: u64 = 30_000;
/// Consecutive flush failures before circuit opens.
const CIRCUIT_AT: u32 = 8;

pub async fn run_persist_worker(consumer: Consumer<PersistEvent>, client: Client, shard_id: u32) {
    run_persist_worker_with_shutdown(consumer, client, shard_id, None).await
}

/// Same as `run_persist_worker` but polls a shutdown flag
/// each flush cycle. When the flag flips to `true`, the
/// worker drains any pending events with one final flush
/// attempt and returns. Used by the risk Main role so that
/// a demote (lease loss) can cleanly stop the worker before
/// `run_main` re-acquires the advisory lock and spawns a fresh
/// one — otherwise a demote → re-acquire cycle leaks worker
/// threads, each holding its own PG connection.
pub async fn run_persist_worker_with_shutdown(
    mut consumer: Consumer<PersistEvent>,
    mut client: Client,
    shard_id: u32,
    shutdown: Option<Arc<AtomicBool>>,
) {
    let mut pending = Vec::with_capacity(1024);
    let mut consec_errors: u32 = 0;
    let mut backoff_ms: u64 = BACKOFF_INIT_MS;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_INTERVAL_MS)).await;

        let stopping = shutdown
            .as_ref()
            .map(|s| s.load(Ordering::Relaxed))
            .unwrap_or(false);

        if pending.is_empty() {
            while let Ok(event) = consumer.pop() {
                pending.push(event);
            }
        }

        if pending.is_empty() {
            if stopping {
                info!(
                    "persist worker shutdown signal received; \
                     no pending events, exiting",
                );
                return;
            }
            continue;
        }

        match flush_batch(&mut client, shard_id, &pending).await {
            Ok(()) => {
                consec_errors = 0;
                backoff_ms = BACKOFF_INIT_MS;
                pending.clear();
                if stopping {
                    info!(
                        "persist worker shutdown signal \
                         received; final flush succeeded, \
                         exiting",
                    );
                    return;
                }
            }
            Err(e) => {
                consec_errors += 1;
                if consec_errors >= CIRCUIT_AT {
                    error!(
                        "persist circuit open: {} consecutive \
                         flush failures; stopping worker: {e}",
                        consec_errors,
                    );
                    break;
                }
                if stopping {
                    warn!(
                        "persist worker shutdown signal \
                         received during error retry; \
                         dropping {} pending events: {e}",
                        pending.len(),
                    );
                    return;
                }
                // ±20% jitter on exponential backoff
                let jitter = backoff_ms as f64 * (0.8 + 0.4 * rand_jitter());
                warn!(
                    "persist flush error ({}/{CIRCUIT_AT}), \
                     retry in {:.0}ms: {e}",
                    consec_errors, jitter,
                );
                tokio::time::sleep(std::time::Duration::from_millis(jitter as u64)).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
            }
        }
    }
}

/// Cheap pseudo-random float in [0, 1) using thread
/// timestamp bits — avoids a rand dep on hot path.
#[inline]
fn rand_jitter() -> f64 {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    (ns % 1000) as f64 / 1000.0
}
