use crate::wal::fan_out;
use crate::wal::CastPeer;
use rsx_book::book::Orderbook;
use rsx_cast::wal::WalWriter;
use rsx_messages::ConfigAppliedRecord;
use rsx_types::time_utils::time_ms;
use rsx_types::time_utils::time_ns;
use rsx_types::SymbolConfig;
use rtrb::Consumer;
use rtrb::Producer;
use rtrb::RingBuffer;
use std::env;
use std::io;
use std::time::Duration;
use std::time::Instant;
use tokio_postgres::error::SqlState;
use tokio_postgres::Client;
use tokio_postgres::Row;
use tracing::info;
use tracing::warn;

/// A scheduled config row: the `SymbolConfig` itself plus its schedule
/// metadata. Composes `SymbolConfig` rather than re-declaring its fields,
/// so there is no conversion step — callers read `.config` directly.
#[derive(Debug, Clone)]
pub struct ScheduledConfig {
    pub config: SymbolConfig,
    pub config_version: u64,
    pub effective_at_ms: u64,
}

/// The config the book boots with, and where it came from.
#[derive(Debug, Clone, Copy)]
pub struct BootConfig {
    pub config: SymbolConfig,
    /// Version of the `symbol_config_applied` row `config` was read from;
    /// `None` when `config` is the env config.
    pub applied_version: Option<u64>,
}

impl BootConfig {
    pub fn env(config: SymbolConfig) -> Self {
        Self {
            config,
            applied_version: None,
        }
    }
}

/// Pick the boot config (`specs/2/19-metadata.md` § Cold start bootstrap):
/// the symbol's `symbol_config_applied` row when one exists, else
/// `env_config`. A missing `symbol_config_applied` table counts as no row
/// (risk creates it and may not have migrated yet). Any other query failure
/// is an error: booting on the env config would run a tick/lot the exchange
/// may have already moved past.
pub async fn boot_config(client: &Client, env_config: SymbolConfig) -> io::Result<BootConfig> {
    let applied = load_applied_config(client, env_config.symbol_id).await?;
    Ok(match applied {
        Some(applied) => {
            info!(
                "boot config from applied v{} for symbol {}",
                applied.config_version, env_config.symbol_id
            );
            BootConfig {
                config: applied.config,
                applied_version: Some(applied.config_version),
            }
        }
        None => {
            info!("no applied config, booting on env config");
            BootConfig::env(env_config)
        }
    })
}

pub async fn poll_scheduled_configs(
    client: &Client,
    symbol_id: u32,
    current_version: u64,
    now_ms: u64,
) -> io::Result<Vec<ScheduledConfig>> {
    let rows = client
        .query(
            "SELECT config_version, effective_at_ms, tick_size, \
             lot_size, price_decimals, qty_decimals \
             FROM symbol_config_schedule \
             WHERE symbol_id = $1 \
               AND config_version > $2 \
               AND effective_at_ms <= $3 \
             ORDER BY config_version ASC",
            &[
                &(symbol_id as i32),
                &(current_version as i64),
                &(now_ms as i64),
            ],
        )
        .await
        .map_err(|e| io::Error::other(format!("poll_scheduled_configs: {}", e)))?;

    Ok(rows
        .iter()
        .map(|row| scheduled_config(symbol_id, row))
        .collect())
}

/// Map a row of `config_version, effective_at_ms, tick_size, lot_size,
/// price_decimals, qty_decimals` to a `ScheduledConfig`.
fn scheduled_config(symbol_id: u32, row: &Row) -> ScheduledConfig {
    ScheduledConfig {
        config: SymbolConfig {
            symbol_id,
            tick_size: row.get(2),
            lot_size: row.get(3),
            price_decimals: row.get::<_, i16>(4) as u8,
            qty_decimals: row.get::<_, i16>(5) as u8,
        },
        config_version: row.get::<_, i64>(0) as u64,
        effective_at_ms: row.get::<_, i64>(1) as u64,
    }
}

pub async fn write_applied_config(
    client: &Client,
    symbol_id: u32,
    config: &ScheduledConfig,
    applied_at_ns: u64,
) -> io::Result<()> {
    client
        .execute(
            "INSERT INTO symbol_config_applied \
             (symbol_id, config_version, effective_at_ms, \
              applied_at_ns, tick_size, lot_size, \
              price_decimals, qty_decimals, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active') \
             ON CONFLICT (symbol_id) \
             DO UPDATE SET \
               config_version = EXCLUDED.config_version, \
               effective_at_ms = EXCLUDED.effective_at_ms, \
               applied_at_ns = EXCLUDED.applied_at_ns, \
               tick_size = EXCLUDED.tick_size, \
               lot_size = EXCLUDED.lot_size, \
               price_decimals = EXCLUDED.price_decimals, \
               qty_decimals = EXCLUDED.qty_decimals, \
               status = EXCLUDED.status",
            &[
                &(symbol_id as i32),
                &(config.config_version as i64),
                &(config.effective_at_ms as i64),
                &(applied_at_ns as i64),
                &config.config.tick_size,
                &config.config.lot_size,
                &(config.config.price_decimals as i16),
                &(config.config.qty_decimals as i16),
            ],
        )
        .await
        .map_err(|e| io::Error::other(format!("write_applied_config: {}", e)))?;

    info!(
        "wrote applied config v{} for symbol {}",
        config.config_version, symbol_id
    );
    Ok(())
}

pub async fn load_applied_config(
    client: &Client,
    symbol_id: u32,
) -> io::Result<Option<ScheduledConfig>> {
    let row = client
        .query_opt(
            "SELECT config_version, effective_at_ms, tick_size, \
             lot_size, price_decimals, qty_decimals \
             FROM symbol_config_applied \
             WHERE symbol_id = $1",
            &[&(symbol_id as i32)],
        )
        .await;
    let row = match row {
        Ok(row) => row,
        Err(e) if e.code() == Some(&SqlState::UNDEFINED_TABLE) => return Ok(None),
        Err(e) => return Err(io::Error::other(format!("load_applied_config: {}", e))),
    };

    Ok(row.map(|row| scheduled_config(symbol_id, &row)))
}

fn get_env<T: std::str::FromStr>(key: &str) -> io::Result<T> {
    let raw = env::var(key).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing env var {}", key),
        )
    })?;
    raw.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {}: {}", key, raw),
        )
    })
}

pub fn load_config_from_env() -> io::Result<SymbolConfig> {
    let symbol_id = get_env::<u32>("RSX_ME_SYMBOL_ID")?;
    let price_decimals = get_env::<u8>("RSX_ME_PRICE_DECIMALS")?;
    let qty_decimals = get_env::<u8>("RSX_ME_QTY_DECIMALS")?;
    let tick_size = get_env::<i64>("RSX_ME_TICK_SIZE")?;
    let lot_size = get_env::<i64>("RSX_ME_LOT_SIZE")?;
    Ok(SymbolConfig {
        symbol_id,
        price_decimals,
        qty_decimals,
        tick_size,
        lot_size,
    })
}

pub fn emit_config_applied(
    wal: &mut WalWriter,
    risk_sender: &mut CastPeer,
    mkt_sender: &mut CastPeer,
    symbol_id: u32,
    config_version: u64,
    effective_at_ms: u64,
) {
    let ts = time_ns();
    let mut record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id,
        _pad0: 0,
        config_version,
        effective_at_ms,
        applied_at_ns: ts,
    };
    fan_out(wal, risk_sender, mkt_sender, &mut record).expect(
        "wal append failed (config_applied) — violates 6-consistency.md invariant 7; CONFIG_APPLIED must precede cast fan-out",
    );
    info!(
        "emitted config_applied v{} for symbol {}",
        config_version, symbol_id,
    );
}

/// Slots in each hand-off ring. A poll returns the few configs that came
/// due in one interval, so the rings do not fill in practice.
pub const CONFIG_RING_CAP: usize = 64;

/// How often the sidecar polls `symbol_config_schedule`
/// (`specs/2/21-orderbook.md` § 2.9).
pub const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(600);

/// Longest the sidecar sleeps between drains of the applied ring. Each
/// wakeup is a context switch, so it stays coarse.
const APPLIED_DRAIN_TICK: Duration = Duration::from_secs(1);

/// A config the loop has applied and the time it did, for the
/// `symbol_config_applied` upsert.
pub type AppliedConfig = (ScheduledConfig, u64);

/// The matching loop's end of the config hand-off: due configs arrive on
/// `due`; each applied one goes back on `applied`. Neither ring blocks.
pub struct ConfigLoopEnd {
    pub due: Consumer<ScheduledConfig>,
    pub applied: Producer<AppliedConfig>,
}

/// The sidecar's end of the config hand-off.
pub struct ConfigSidecarEnd {
    pub due: Producer<ScheduledConfig>,
    pub applied: Consumer<AppliedConfig>,
}

/// Create both ends of the config hand-off.
pub fn config_link() -> (ConfigLoopEnd, ConfigSidecarEnd) {
    let (due_producer, due_consumer) = RingBuffer::new(CONFIG_RING_CAP);
    let (applied_producer, applied_consumer) = RingBuffer::new(CONFIG_RING_CAP);
    (
        ConfigLoopEnd {
            due: due_consumer,
            applied: applied_producer,
        },
        ConfigSidecarEnd {
            due: due_producer,
            applied: applied_consumer,
        },
    )
}

/// Apply every config the sidecar has handed over: update the book, queue
/// the applied-row write, emit `CONFIG_APPLIED`. Called by the pinned loop
/// each spin; it does ring pops and WAL appends only, no I/O to Postgres.
pub fn apply_due_configs(
    link: &mut ConfigLoopEnd,
    book: &mut Orderbook,
    wal_writer: &mut WalWriter,
    cast_sender: &mut CastPeer,
    mkt_sender: &mut CastPeer,
    symbol_id: u32,
) {
    while let Ok(cfg) = link.due.pop() {
        book.update_config(cfg.config);
        let config_version = cfg.config_version;
        let effective_at_ms = cfg.effective_at_ms;
        if link.applied.push((cfg, time_ns())).is_err() {
            warn!("applied-config ring full: applied row for v{config_version} not written");
        }
        emit_config_applied(
            wal_writer,
            cast_sender,
            mkt_sender,
            symbol_id,
            config_version,
            effective_at_ms,
        );
    }
}

/// Query the schedule once and push each due config onto `due`. Returns the
/// highest version handed over, or `config_version` when none was.
async fn hand_off_due_configs(
    client: &Client,
    symbol_id: u32,
    config_version: u64,
    due: &mut Producer<ScheduledConfig>,
) -> u64 {
    let configs = match poll_scheduled_configs(client, symbol_id, config_version, time_ms()).await {
        Ok(configs) => configs,
        Err(e) => {
            warn!("config poll failed: {}", e);
            return config_version;
        }
    };
    let mut handed = config_version;
    for cfg in configs {
        let version = cfg.config_version;
        if due.push(cfg).is_err() {
            warn!("config ring full: v{version} stays scheduled until the next poll");
            break;
        }
        handed = version;
    }
    handed
}

/// Upsert the applied row of every config the loop has applied.
async fn write_applied_configs(
    client: &Client,
    symbol_id: u32,
    applied: &mut Consumer<AppliedConfig>,
) {
    while let Ok((cfg, applied_at_ns)) = applied.pop() {
        if let Err(e) = write_applied_config(client, symbol_id, &cfg, applied_at_ns).await {
            warn!("matching: write_applied_config failed: {e}");
        }
    }
}

/// The config sidecar: owns the Postgres client and does every config
/// round trip so the pinned loop never waits on one. Polls the schedule
/// every `poll_interval` for versions above `config_version`, hands the due
/// ones to the loop and writes the applied rows the loop reports back.
/// Returns when the loop drops its end. The caller supplies the runtime.
pub async fn run_config_sidecar(
    client: Client,
    symbol_id: u32,
    mut config_version: u64,
    poll_interval: Duration,
    mut link: ConfigSidecarEnd,
) {
    let mut next_poll = Instant::now() + poll_interval;
    while !link.due.is_abandoned() {
        write_applied_configs(&client, symbol_id, &mut link.applied).await;
        if Instant::now() >= next_poll {
            config_version =
                hand_off_due_configs(&client, symbol_id, config_version, &mut link.due).await;
            next_poll = Instant::now() + poll_interval;
        }
        let until_poll = next_poll.saturating_duration_since(Instant::now());
        tokio::time::sleep(until_poll.min(APPLIED_DRAIN_TICK)).await;
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
