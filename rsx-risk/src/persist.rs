use crate::account::Account;
use crate::config::load_symbol_config;
use crate::config::SymbolRiskConfig;
use crate::db::connect;
use crate::insurance::InsuranceFund;
use crate::position::Position;
use crate::risk_utils::rand_jitter;
use crate::shard::RiskShard;
use rtrb::Consumer;
use rtrb::Producer;
use rtrb::PushError;
use std::sync::atomic::AtomicU8;
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
    FundingSettlement(FundingSettlement),
    InsuranceFund(InsuranceFund),
    Liquidation(LiquidationRecord),
    FrozenInsert(FrozenOrderRecord),
    FrozenRemove {
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    /// Fetch `symbol_config_schedule` row `(symbol_id, config_version)`
    /// and hand it back to the shard over the config ring.
    ConfigReload {
        symbol_id: u32,
        config_version: u64,
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
    pub interval_id: u64,
    pub amount: i64,
    pub rate: i64,
    pub settlement_ts: u64,
}

/// One symbol's settlement of one funding interval. It is a single
/// persist event so the payments, the collateral they changed and the
/// settled-interval record commit in one transaction or not at all.
#[derive(Clone, Debug)]
pub struct FundingSettlement {
    pub symbol_id: u32,
    pub interval_id: u64,
    pub payments: Vec<FundingRecord>,
    pub accounts: Vec<Account>,
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
               last_fill_seq = $8, version = $9 \
             WHERE positions.version <= EXCLUDED.version",
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
               collateral = $2, version = $3 \
             WHERE accounts.version <= EXCLUDED.version",
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
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) \
             ON CONFLICT (symbol_id, seq) DO NOTHING",
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
             DO UPDATE SET \
               last_seq = GREATEST(tips.last_seq, EXCLUDED.last_seq)",
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
             (user_id, symbol_id, interval_id, amount, rate, \
              settlement_ts) \
             VALUES ($1,$2,$3,$4,$5,$6)",
            &[
                &(p.user_id as i32),
                &(p.symbol_id as i32),
                &(p.interval_id as i64),
                &p.amount,
                &p.rate,
                &(p.settlement_ts as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

/// Record `(symbol_id, interval_id)` as settled for `shard_id`. A
/// second record of the same interval violates the primary key and
/// fails the transaction carrying it.
pub async fn insert_funding_intervals(
    tx: &tokio_postgres::Transaction<'_>,
    shard_id: u32,
    settled: &[(u32, u64)],
) -> Result<(), Error> {
    for &(symbol_id, interval_id) in settled {
        tx.execute(
            "INSERT INTO funding_intervals \
             (shard_id, symbol_id, interval_id) \
             VALUES ($1,$2,$3)",
            &[
                &(shard_id as i32),
                &(symbol_id as i32),
                &(interval_id as i64),
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
             DO UPDATE SET balance = $2, version = $3 \
             WHERE insurance_fund.version <= EXCLUDED.version",
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

/// Writes one batch in one transaction. With `lease_pid` (the
/// backend pid of the session holding the shard's advisory lock)
/// the transaction aborts unless that session still holds the
/// lock, so a demoted main cannot commit.
pub async fn flush_batch(
    client: &mut Client,
    shard_id: u32,
    events: &[PersistEvent],
    lease_pid: Option<i32>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if events.is_empty() {
        return Ok(());
    }

    let mut positions = Vec::new();
    let mut accounts = Vec::new();
    let mut fills = Vec::new();
    let mut tips = Vec::new();
    let mut funding = Vec::new();
    let mut settled_intervals = Vec::new();
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
            PersistEvent::FundingSettlement(settlement) => {
                accounts.extend(settlement.accounts.iter().cloned());
                funding.extend(settlement.payments.iter().cloned());
                settled_intervals.push((settlement.symbol_id, settlement.interval_id));
            }
            PersistEvent::InsuranceFund(fund) => insurance_funds.push(fund.clone()),
            PersistEvent::Liquidation(liq) => liquidations.push(liq.clone()),
            PersistEvent::FrozenInsert(f) => frozen_inserts.push(f.clone()),
            PersistEvent::FrozenRemove {
                user_id,
                order_id_hi,
                order_id_lo,
            } => frozen_removes.push((*user_id, *order_id_hi, *order_id_lo)),
            PersistEvent::ConfigReload { .. } => {}
        }
    }

    let tx = client.transaction().await?;
    if let Some(pid) = lease_pid {
        let held = tx
            .query_opt(
                "SELECT 1 FROM pg_locks WHERE locktype = 'advisory' \
                 AND objid::bigint = $1 AND pid = $2 AND granted",
                &[&(shard_id as i64), &pid],
            )
            .await?;
        if held.is_none() {
            return Err(format!("advisory lock {shard_id} not held by session {pid}").into());
        }
    }
    upsert_positions(&tx, &positions).await?;
    upsert_accounts(&tx, &accounts).await?;
    upsert_frozen_orders(&tx, &frozen_inserts).await?;
    delete_frozen_orders(&tx, &frozen_removes).await?;
    insert_fills(&tx, &fills).await?;
    upsert_tips(&tx, shard_id, &tips).await?;
    insert_funding(&tx, &funding).await?;
    insert_funding_intervals(&tx, shard_id, &settled_intervals).await?;
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

/// How the persist worker ends, stored in an `AtomicU8`.
/// `FlushAndStop`: the shard lock is still held, so pending
/// events get one final flush. `DropAndStop`: the lock is lost
/// or going; pending events are dropped and the next main
/// replays them from the persisted tip.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopMode {
    Run = 0,
    FlushAndStop = 1,
    DropAndStop = 2,
}

impl StopMode {
    fn load(stop: &AtomicU8) -> StopMode {
        match stop.load(Ordering::Relaxed) {
            1 => StopMode::FlushAndStop,
            2 => StopMode::DropAndStop,
            _ => StopMode::Run,
        }
    }
}

pub async fn run_persist_worker(consumer: Consumer<PersistEvent>, client: Client, shard_id: u32) {
    run_persist_worker_with_shutdown(consumer, client, shard_id, None, None, None).await
}

/// Same as `run_persist_worker` but polls a `StopMode` each
/// flush cycle. Used by the live node so that a demote can stop
/// the worker before the node re-acquires the advisory lock and
/// spawns a fresh one — otherwise a demote → re-acquire cycle
/// leaks worker threads, each holding its own PG connection.
/// With `lease_pid`, every flush is fenced on the lock session.
///
/// `ConfigReload` events are answered inline: the row is read
/// from `symbol_config_schedule` and pushed onto `configs`, the
/// ring the shard drains on its next tick.
pub async fn run_persist_worker_with_shutdown(
    mut consumer: Consumer<PersistEvent>,
    mut client: Client,
    shard_id: u32,
    shutdown: Option<Arc<AtomicU8>>,
    mut configs: Option<Producer<SymbolRiskConfig>>,
    lease_pid: Option<i32>,
) {
    let mut pending = Vec::with_capacity(1024);
    let mut consec_errors: u32 = 0;
    let mut backoff_ms: u64 = BACKOFF_INIT_MS;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_INTERVAL_MS)).await;

        let mode = shutdown
            .as_ref()
            .map_or(StopMode::Run, |stop| StopMode::load(stop));
        if mode == StopMode::DropAndStop {
            warn!("persist worker stopping without a final flush");
            return;
        }
        let stopping = mode == StopMode::FlushAndStop;

        if pending.is_empty() {
            while let Ok(event) = consumer.pop() {
                if let PersistEvent::ConfigReload {
                    symbol_id,
                    config_version,
                } = event
                {
                    reload_symbol_config(&client, symbol_id, config_version, &mut configs).await;
                    continue;
                }
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

        match flush_batch(&mut client, shard_id, &pending, lease_pid).await {
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

impl RiskShard {
    /// Push one event onto the persist ring. WAL.md hard
    /// backpressure: ring full -> the producer STALLS (bare
    /// busy-spin; the worker drains from its own thread) — never
    /// drop. A dropped Fill followed by a successful Tip push
    /// would advance the PG tip past the fill: replay-from-tip
    /// could never re-cover it and the fills table keeps a
    /// permanent hole. The only exit without delivery is a gone
    /// worker (consumer dropped after circuit-open): those events
    /// ARE re-covered by construction — the PG tip only advances
    /// through this same worker, so a restart replays from the
    /// frozen tip. The live loop demotes on worker death.
    pub(crate) fn push_persist(&mut self, event: PersistEvent) {
        let Some(ref mut p) = self.persist_producer else {
            return;
        };
        let mut event = event;
        loop {
            match p.push(event) {
                Ok(()) => {
                    if self.persist_stalled {
                        info!("persist ring drained, resuming");
                        self.persist_stalled = false;
                    }
                    return;
                }
                Err(PushError::Full(returned)) => {
                    if p.is_abandoned() {
                        error!(
                            "persist worker gone; dropping event \
                             (re-covered by replay from persisted tip)"
                        );
                        return;
                    }
                    // Warn once per stall episode, then spin
                    // silently — this runs on the pinned core.
                    if !self.persist_stalled {
                        warn!("persist ring full, stalling producer");
                        self.persist_stalled = true;
                    }
                    event = returned;
                }
            }
        }
    }

    /// Occupancy of the persist ring (used slots), published to
    /// the `/metrics` `persist_ring` gauge each loop iteration.
    pub fn persist_ring_used(&self) -> u64 {
        match &self.persist_producer {
            Some(p) => (p.buffer().capacity() - p.slots()) as u64,
            None => 0,
        }
    }
}

/// Spawn the persist worker thread. It owns a dedicated
/// current-thread tokio runtime + PG connection and drains the
/// persist ring until `shutdown` leaves `StopMode::Run`. Stop it
/// with `stop_persist_worker` before any transition out of Live so
/// the next promote spawns a fresh worker without doubling up on
/// PG connections. `lease_pid` is the backend pid of the session
/// holding the shard's advisory lock.
pub fn spawn_persist_worker(
    url: String,
    shard_id: u32,
    consumer: Consumer<PersistEvent>,
    configs: Producer<SymbolRiskConfig>,
    shutdown: Arc<AtomicU8>,
    lease_pid: i32,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        persist_thread_body(url, shard_id, consumer, configs, shutdown, lease_pid)
    })
}

fn persist_thread_body(
    url: String,
    shard_id: u32,
    consumer: Consumer<PersistEvent>,
    configs: Producer<SymbolRiskConfig>,
    shutdown: Arc<AtomicU8>,
    lease_pid: i32,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio rt");
    rt.block_on(async move {
        let client = connect(&url).await.expect("pg connect for persist");
        run_persist_worker_with_shutdown(
            consumer,
            client,
            shard_id,
            Some(shutdown),
            Some(configs),
            Some(lease_pid),
        )
        .await;
    });
}

async fn reload_symbol_config(
    client: &Client,
    symbol_id: u32,
    config_version: u64,
    configs: &mut Option<Producer<SymbolRiskConfig>>,
) {
    let Some(configs) = configs else {
        error!("config reload requested with no config ring: symbol={symbol_id} v{config_version}");
        return;
    };
    let loaded = load_symbol_config(client, symbol_id, config_version).await;
    let config = match loaded {
        Ok(Some(config)) => config,
        Ok(None) => {
            error!("CONFIG_APPLIED v{config_version} for symbol {symbol_id} has no schedule row");
            return;
        }
        Err(e) => {
            error!("config reload failed: symbol={symbol_id} v{config_version}: {e}");
            return;
        }
    };
    if configs.push(config).is_err() {
        error!("config ring full; dropping reload of symbol {symbol_id} v{config_version}");
    }
}

/// Signal the persist worker to stop in `mode` and wait up to 5s
/// for the thread to exit — well past the worst-case backoff
/// between failed flushes. A worker still running after that is
/// abandoned (cleaned up on process exit) so a stuck worker cannot
/// hang a demote.
pub fn stop_persist_worker(
    shutdown: &Arc<AtomicU8>,
    mode: StopMode,
    handle: std::thread::JoinHandle<()>,
) {
    shutdown.store(mode as u8, Ordering::Relaxed);
    let start = std::time::Instant::now();
    loop {
        if handle.is_finished() {
            if let Err(e) = handle.join() {
                warn!("persist worker thread panicked: {:?}", e,);
            }
            info!("persist worker stopped");
            return;
        }
        if start.elapsed() > std::time::Duration::from_secs(5) {
            warn!(
                "persist worker did not exit within 5s; \
                 abandoning thread (will be cleaned up on \
                 process exit)",
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
