//! The gateway's symbol table: tick, lot, decimals, status and minimum
//! notional per symbol, read from risk's Postgres config
//! (`symbol_config_schedule`). Risk owns the schema and runs its migrations;
//! the gateway only reads.
//!
//! The active row of a symbol is the highest `config_version` whose
//! `effective_at_ms` is at or before now, risk's own rule
//! (`rsx_risk::config::load_symbol_configs`). The table is loaded once at
//! boot, before the reactor exists. A sidecar thread then re-reads it on a
//! poll and on a request from the reactor (a `CONFIG_APPLIED` newer than the
//! table) and hands a changed table back over an SPSC ring; the reactor never
//! waits on Postgres.

use rsx_types::time_utils::time_ms;
use rsx_types::SymbolConfig;
use rtrb::Consumer;
use rtrb::Producer;
use rtrb::RingBuffer;
use rustc_hash::FxHashMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
use tokio::sync::Notify;
use tokio_postgres::error::SqlState;
use tokio_postgres::Client;
use tokio_postgres::NoTls;
use tracing::info;
use tracing::warn;

#[cfg(test)]
#[path = "symbols_test.rs"]
mod symbols_test;

const SCHEMA_RETRY: Duration = Duration::from_millis(500);

const UPDATE_RING_CAPACITY: usize = 4;

/// One symbol's active config row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolEntry {
    pub config: SymbolConfig,
    pub config_version: u64,
    /// The row's `status` text, served to clients as is; the gateway never
    /// checks it (matching and risk own symbol status).
    pub status: String,
    /// Smallest order, in `price * qty` raw units. Served to clients; risk
    /// enforces it.
    pub min_notional: i64,
}

impl SymbolEntry {
    /// An entry for `config` at version 0, status `trading`, minimum 1.
    pub fn trading(config: SymbolConfig) -> Self {
        Self {
            config,
            config_version: 0,
            status: "trading".to_string(),
            min_notional: 1,
        }
    }
}

/// Active entry per symbol id.
pub type SymbolTable = FxHashMap<u32, SymbolEntry>;

/// A table of `configs`, each as a `trading` entry.
pub fn build_table(configs: impl IntoIterator<Item = SymbolConfig>) -> SymbolTable {
    configs
        .into_iter()
        .map(|config| (config.symbol_id, SymbolEntry::trading(config)))
        .collect()
}

/// The entries of `table` in symbol id order.
pub fn sort_entries(table: &SymbolTable) -> Vec<&SymbolEntry> {
    let mut entries: Vec<&SymbolEntry> = table.values().collect();
    entries.sort_by_key(|entry| entry.config.symbol_id);
    entries
}

fn invalid(symbol_id: i32, config_version: i64, what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("symbol {symbol_id} v{config_version}: {what}"),
    )
}

fn is_status_token(status: &str) -> bool {
    !status.is_empty()
        && status
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn entry_from_row(row: &tokio_postgres::Row) -> io::Result<SymbolEntry> {
    let symbol_id: i32 = row.get(0);
    let config_version: i64 = row.get(1);
    let bad = |what: &str| invalid(symbol_id, config_version, what);
    let tick_size: i64 = row.get(2);
    let lot_size: i64 = row.get(3);
    if symbol_id < 0 {
        return Err(bad("negative symbol id"));
    }
    if config_version < 0 {
        return Err(bad("negative config_version"));
    }
    if tick_size <= 0 {
        return Err(bad(&format!("tick_size is {tick_size}, must be > 0")));
    }
    if lot_size <= 0 {
        return Err(bad(&format!("lot_size is {lot_size}, must be > 0")));
    }
    let price_decimals =
        u8::try_from(row.get::<_, i16>(4)).map_err(|_| bad("price_decimals outside 0..=255"))?;
    let qty_decimals =
        u8::try_from(row.get::<_, i16>(5)).map_err(|_| bad("qty_decimals outside 0..=255"))?;
    let status: String = row.get(6);
    if !is_status_token(&status) {
        return Err(bad(&format!("status {status:?} is not a lowercase word")));
    }
    let min_notional = match row.get::<_, Option<i64>>(7) {
        Some(min_notional) if min_notional > 0 => min_notional,
        Some(min_notional) => {
            return Err(bad(&format!("min_notional is {min_notional}, must be > 0")))
        }
        None => return Err(bad("min_notional is NULL")),
    };
    Ok(SymbolEntry {
        config: SymbolConfig {
            symbol_id: symbol_id as u32,
            price_decimals,
            qty_decimals,
            tick_size,
            lot_size,
        },
        config_version: config_version as u64,
        status,
        min_notional,
    })
}

/// The active entry of every symbol at `now_ms`. A missing table or no
/// active row is `NotFound` (risk may not have migrated yet); a malformed
/// row is `InvalidData` and fails the whole load.
pub async fn load_symbol_table(client: &Client, now_ms: u64) -> io::Result<SymbolTable> {
    let rows = client
        .query(
            "SELECT DISTINCT ON (symbol_id) symbol_id, config_version, \
             tick_size, lot_size, price_decimals, qty_decimals, status, \
             min_notional \
             FROM symbol_config_schedule \
             WHERE effective_at_ms <= $1 \
             ORDER BY symbol_id, config_version DESC",
            &[&(now_ms as i64)],
        )
        .await
        .map_err(|error| {
            if error.code() == Some(&SqlState::UNDEFINED_TABLE) {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "symbol_config_schedule does not exist (risk has not migrated)",
                )
            } else {
                io::Error::other(format!("load_symbol_table: {error}"))
            }
        })?;
    if rows.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "symbol_config_schedule has no active row",
        ));
    }
    let mut table = SymbolTable::default();
    for row in &rows {
        let entry = entry_from_row(row)?;
        table.insert(entry.config.symbol_id, entry);
    }
    Ok(table)
}

async fn drive_pg_connection(
    connection: tokio_postgres::Connection<
        tokio_postgres::Socket,
        tokio_postgres::tls::NoTlsStream,
    >,
) {
    if let Err(error) = connection.await {
        warn!("symbols: postgres connection ended: {error}");
    }
}

async fn connect(url: &str) -> io::Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .map_err(|error| io::Error::other(format!("connect to postgres: {error}")))?;
    tokio::spawn(drive_pg_connection(connection));
    Ok(client)
}

async fn load_when_migrated(client: &Client, schema_wait: Duration) -> io::Result<SymbolTable> {
    let deadline = Instant::now() + schema_wait;
    loop {
        match load_symbol_table(client, time_ms()).await {
            Err(error) if error.kind() == io::ErrorKind::NotFound && Instant::now() < deadline => {
                warn!("symbols: {error}, retrying");
                tokio::time::sleep(SCHEMA_RETRY).await;
            }
            outcome => return outcome,
        }
    }
}

/// The connection and runtime of the boot load, kept for the sidecar.
pub struct SymbolSource {
    pub table: SymbolTable,
    runtime: Runtime,
    client: Client,
    url: String,
}

/// Connect and load the table before the reactor exists. An unreachable
/// database, an unreadable table and a malformed row are errors, and so is
/// a schema that is still missing or empty after `schema_wait`; the caller
/// stops boot.
pub fn boot_load(url: &str, schema_wait: Duration) -> io::Result<SymbolSource> {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let (client, table) = runtime.block_on(async {
        let client = connect(url).await?;
        let table = load_when_migrated(&client, schema_wait).await?;
        io::Result::Ok((client, table))
    })?;
    Ok(SymbolSource {
        runtime,
        client,
        url: url.to_string(),
        table,
    })
}

/// The reactor's end of the sidecar.
pub struct SymbolSidecar {
    /// Ask the sidecar to re-read the table now.
    pub reload: Arc<Notify>,
    /// Changed tables, oldest first; the last one is current.
    pub updates: Consumer<SymbolTable>,
}

/// Start the sidecar thread. It owns the runtime and the Postgres client,
/// re-reads the table every `poll_interval` and on each `reload`, and
/// publishes it when it differs from the last one it published. A failed read
/// is logged and the old table stays in force; the connection is reopened on
/// the next cycle. The thread ends when the reactor drops `updates`.
pub fn spawn_sidecar(source: SymbolSource, poll_interval: Duration) -> io::Result<SymbolSidecar> {
    let reload = Arc::new(Notify::new());
    let (producer, updates) = RingBuffer::new(UPDATE_RING_CAPACITY);
    let thread_reload = reload.clone();
    std::thread::Builder::new()
        .name("gw-symbols".into())
        .spawn(move || {
            let SymbolSource {
                runtime,
                client,
                url,
                table,
            } = source;
            runtime.block_on(sidecar_loop(
                url,
                client,
                table,
                poll_interval,
                thread_reload,
                producer,
            ));
        })?;
    Ok(SymbolSidecar { reload, updates })
}

async fn sidecar_loop(
    url: String,
    client: Client,
    mut published: SymbolTable,
    poll_interval: Duration,
    reload: Arc<Notify>,
    mut updates: Producer<SymbolTable>,
) {
    let mut client = Some(client);
    while !updates.is_abandoned() {
        tokio::select! {
            () = reload.notified() => {}
            () = tokio::time::sleep(poll_interval) => {}
        }
        if client.is_none() {
            match connect(&url).await {
                Ok(fresh) => client = Some(fresh),
                Err(error) => {
                    warn!("symbols: {error}");
                    continue;
                }
            }
        }
        let Some(open) = client.as_ref() else {
            continue;
        };
        match load_symbol_table(open, time_ms()).await {
            Ok(table) if table != published => match updates.push(table.clone()) {
                Ok(()) => published = table,
                Err(_) => warn!("symbols: update ring full, retrying next cycle"),
            },
            Ok(_) => {}
            Err(error) => {
                warn!("symbols: reload failed, keeping the current table: {error}");
                client = None;
            }
        }
    }
}

/// Log each entry of `table` that is new or differs from `old`.
pub fn log_changes(old: &SymbolTable, table: &SymbolTable) {
    for entry in sort_entries(table) {
        if old.get(&entry.config.symbol_id) == Some(entry) {
            continue;
        }
        info!(
            "gateway symbol_config sid={} v={} status={} tick_size={} lot_size={} price_decimals={} qty_decimals={} min_notional={}",
            entry.config.symbol_id,
            entry.config_version,
            entry.status,
            entry.config.tick_size,
            entry.config.lot_size,
            entry.config.price_decimals,
            entry.config.qty_decimals,
            entry.min_notional,
        );
    }
}
