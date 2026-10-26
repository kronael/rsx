use crate::check::Check;
use crate::check::FUNDING;
use crate::check::LOCKS;
use crate::persisted::tally_funding;
use crate::persisted::tally_locks;
use crate::persisted::IntervalSum;
use crate::persisted::Persisted;
use rsx_matching::config::load_applied_config;
use rsx_matching::config::poll_scheduled_configs;
use rsx_matching::config::BootConfig;
use rsx_matching::config::ScheduledConfig;
use rsx_risk::db::connect;
use rsx_risk::Position;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
use tokio_postgres::Client;

const FUNDING_SUMS_SQL: &str = "SELECT symbol_id, interval_id, SUM(amount)::text \
     FROM funding WHERE interval_id IS NOT NULL \
     GROUP BY symbol_id, interval_id ORDER BY symbol_id, interval_id";

const FUNDING_SHARDS_SQL: &str = "SELECT symbol_id, interval_id, COUNT(DISTINCT shard_id) \
     FROM funding_intervals GROUP BY symbol_id, interval_id";

const SHARD_COUNT_SQL: &str = "SELECT COUNT(DISTINCT shard_id) FROM funding_intervals";

const SHARD_LOCKS_SQL: &str = "SELECT objid::int8, COUNT(DISTINCT pid) \
     FROM pg_locks WHERE locktype = 'advisory' AND granted AND classid = 0 \
     GROUP BY objid ORDER BY objid";

/// A Postgres connection with the runtime that drives it. Every call blocks
/// until its queries finish.
pub struct Pg {
    runtime: Runtime,
    client: Client,
}

fn describe(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Pg {
    pub fn connect(url: &str) -> Result<Pg, String> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(describe)?;
        let client = runtime
            .block_on(connect(url))
            .map_err(|error| format!("failed to connect to postgres: {}", error))?;
        Ok(Pg { runtime, client })
    }

    /// Positions and tips risk has persisted.
    pub fn load_persisted(&self) -> Result<Persisted, String> {
        let positions = self
            .runtime
            .block_on(self.client.query(
                "SELECT user_id, symbol_id, long_qty, short_qty, long_entry_cost, \
                 short_entry_cost, realized_pnl, last_fill_seq, version FROM positions",
                &[],
            ))
            .map_err(describe)?;
        let tips = self
            .runtime
            .block_on(
                self.client
                    .query("SELECT instance_id, symbol_id, last_seq FROM tips", &[]),
            )
            .map_err(describe)?;
        let mut by_key: FxHashMap<(u32, u32), Position> = FxHashMap::default();
        for row in &positions {
            let user_id = row.try_get::<_, i32>(0).map_err(describe)? as u32;
            let symbol_id = row.try_get::<_, i32>(1).map_err(describe)? as u32;
            by_key.insert(
                (user_id, symbol_id),
                Position {
                    user_id,
                    symbol_id,
                    long_qty: row.try_get(2).map_err(describe)?,
                    short_qty: row.try_get(3).map_err(describe)?,
                    long_entry_cost: row.try_get(4).map_err(describe)?,
                    short_entry_cost: row.try_get(5).map_err(describe)?,
                    realized_pnl: row.try_get(6).map_err(describe)?,
                    last_fill_seq: row.try_get::<_, i64>(7).map_err(describe)? as u64,
                    version: row.try_get::<_, i64>(8).map_err(describe)? as u64,
                },
            );
        }
        let mut tip_rows = Vec::with_capacity(tips.len());
        for row in &tips {
            tip_rows.push((
                row.try_get::<_, i32>(0).map_err(describe)? as u32,
                row.try_get::<_, i32>(1).map_err(describe)? as u32,
                row.try_get::<_, i64>(2).map_err(describe)? as u64,
            ));
        }
        Ok(Persisted::new(by_key, tip_rows))
    }

    /// The config the symbol's book runs on: the applied row, else the
    /// newest scheduled row already in force. `None` when the symbol has no
    /// config row.
    pub fn load_boot_config(
        &self,
        symbol_id: u32,
        book_capacity: u32,
    ) -> Result<Option<BootConfig>, String> {
        let applied = self
            .runtime
            .block_on(load_applied_config(&self.client, symbol_id))
            .map_err(describe)?;
        let config: Option<ScheduledConfig> = match applied {
            Some(applied) => Some(applied),
            None => {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(describe)?
                    .as_millis() as u64;
                let scheduled = self
                    .runtime
                    .block_on(poll_scheduled_configs(&self.client, symbol_id, 0, now_ms))
                    .map_err(describe)?;
                scheduled.into_iter().last()
            }
        };
        Ok(config.map(|scheduled| BootConfig {
            config: scheduled.config,
            applied_version: Some(scheduled.config_version),
            book_capacity,
        }))
    }

    fn sum_funding(&self) -> Result<(Vec<IntervalSum>, u64), String> {
        let sums = self
            .runtime
            .block_on(self.client.query(FUNDING_SUMS_SQL, &[]))
            .map_err(describe)?;
        let settled = self
            .runtime
            .block_on(self.client.query(FUNDING_SHARDS_SQL, &[]))
            .map_err(describe)?;
        let shards = self
            .runtime
            .block_on(self.client.query_one(SHARD_COUNT_SQL, &[]))
            .map_err(describe)?;
        let mut shards_settled: BTreeMap<(i32, i64), u64> = BTreeMap::new();
        for row in &settled {
            let key = (
                row.try_get::<_, i32>(0).map_err(describe)?,
                row.try_get::<_, i64>(1).map_err(describe)?,
            );
            shards_settled.insert(key, row.try_get::<_, i64>(2).map_err(describe)? as u64);
        }
        let mut result = Vec::with_capacity(sums.len());
        for row in &sums {
            let symbol_id = row.try_get::<_, i32>(0).map_err(describe)?;
            let interval_id = row.try_get::<_, i64>(1).map_err(describe)?;
            let total = row.try_get::<_, String>(2).map_err(describe)?;
            result.push(IntervalSum {
                symbol_id: symbol_id as u32,
                interval_id: interval_id as u64,
                shards_settled: shards_settled
                    .get(&(symbol_id, interval_id))
                    .copied()
                    .unwrap_or(0),
                total: total.parse::<i128>().map_err(describe)?,
            });
        }
        let shard_count = shards.try_get::<_, i64>(0).map_err(describe)? as u64;
        Ok((result, shard_count))
    }

    pub fn check_funding(&self) -> Check {
        match self.sum_funding() {
            Ok((sums, shards)) => {
                let (tally, unsettled) = tally_funding(&sums, shards);
                let mut check = Check::from_tally(Some(9), FUNDING, "settled intervals", tally);
                if unsettled > 0 {
                    check.detail.push_str(&format!(
                        "; {} intervals not settled by every shard",
                        unsettled
                    ));
                }
                check
            }
            Err(error) => Check::fail(Some(9), FUNDING, error),
        }
    }

    fn count_lock_holders(&self) -> Result<Vec<(i64, i64)>, String> {
        let rows = self
            .runtime
            .block_on(self.client.query(SHARD_LOCKS_SQL, &[]))
            .map_err(describe)?;
        let mut holders = Vec::with_capacity(rows.len());
        for row in &rows {
            holders.push((
                row.try_get::<_, i64>(0).map_err(describe)?,
                row.try_get::<_, i64>(1).map_err(describe)?,
            ));
        }
        Ok(holders)
    }

    pub fn check_locks(&self) -> Check {
        match self.count_lock_holders() {
            Ok(holders) => {
                Check::from_tally(Some(10), LOCKS, "held shard locks", tally_locks(&holders))
            }
            Err(error) => Check::fail(Some(10), LOCKS, error),
        }
    }
}
