use crate::funding::FundingConfig;
use crate::margin::SymbolRiskParams;
use crate::persist::PersistEvent;
use crate::shard::RiskShard;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio_postgres::Client;
use tracing::info;
use tracing::warn;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeStreamConfig {
    pub symbol_id: u32,
    pub cast_addr: SocketAddr,
    pub recv_addr: SocketAddr,
    pub replay_addr: String,
}

pub struct LiquidationConfig {
    pub base_delay_ns: u64,
    pub base_slip_bps: u64,
    pub max_rounds: u32,
    /// How long a position waits before its liquidation cascade
    /// may restart after ending in socialized loss. Re-liquidation
    /// is intended -- the exchange keeps drawing until the position
    /// is covered -- but at minutes, not the seconds the cascade
    /// itself runs at.
    pub socialized_cooldown_ns: u64,
    /// Cap on aggregate slippage (round^2 * base_slip_bps)
    /// applied when generating liquidation orders. Default
    /// 9999 (99.99%); lower values halt runaway slippage sooner.
    pub max_slip_bps: u64,
}

impl Default for LiquidationConfig {
    fn default() -> Self {
        Self {
            base_delay_ns: 100_000_000, // 100ms
            base_slip_bps: 1,
            max_rounds: 10,
            socialized_cooldown_ns: 120_000_000_000, // 2 min
            max_slip_bps: 9999,
        }
    }
}

pub struct ReplicationConfig {
    pub lease_poll_interval_ms: u64,
    pub lease_renew_interval_ms: u64,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
        }
    }
}

/// Sharding, liquidation and lease knobs from the environment.
/// `symbol_params`/fees are indexed by `symbol_id` and start as
/// unconfigured (`None`, zero fees): the active rows of
/// `symbol_config_schedule` fill them through
/// `RiskShard::apply_symbol_config`.
pub struct ShardConfig {
    pub shard_id: u32,
    pub shard_count: u32,
    pub max_symbols: usize,
    pub symbol_params: Vec<Option<SymbolRiskParams>>,
    pub taker_fee_bps: Vec<i64>,
    pub maker_fee_bps: Vec<i64>,
    pub funding_config: FundingConfig,
    pub liquidation_config: LiquidationConfig,
    pub replication_config: ReplicationConfig,
}

/// `key` parsed as `T`, or `default` when unset. A set-but-malformed
/// value is an error: a typo in a shard id or a margin knob must not
/// silently become the default.
fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> io::Result<T> {
    let Ok(raw) = std::env::var(key) else {
        return Ok(default);
    };
    raw.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{key}={raw:?} is not a valid {}",
                std::any::type_name::<T>()
            ),
        )
    })
}

/// Shard config from the environment: `RSX_RISK_SHARD_ID`,
/// `RSX_RISK_SHARD_COUNT`, `RSX_RISK_MAX_SYMBOLS`, the
/// `RSX_LIQUIDATION_*` and `RSX_RISK_LEASE_*` knobs. Symbol risk
/// params are not here; they come from Postgres.
pub fn load_shard_config() -> io::Result<ShardConfig> {
    let shard_id = env_parse("RSX_RISK_SHARD_ID", 0u32)?;
    let shard_count = env_parse("RSX_RISK_SHARD_COUNT", 1u32)?;
    let max_symbols = env_parse("RSX_RISK_MAX_SYMBOLS", 64usize)?;
    if shard_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RSX_RISK_SHARD_COUNT must be > 0",
        ));
    }
    if shard_id >= shard_count {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "RSX_RISK_SHARD_ID ({}) must be < RSX_RISK_SHARD_COUNT ({})",
                shard_id, shard_count
            ),
        ));
    }
    if max_symbols == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RSX_RISK_MAX_SYMBOLS must be > 0",
        ));
    }

    // Long-side liq price is mark*(10000-slip)/10000; slip
    // above 10_000 bps drives it negative. Fail fast.
    let max_slip_bps = env_parse("RSX_LIQUIDATION_MAX_SLIP_BPS", 9999u64)?;
    if max_slip_bps > 10_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "RSX_LIQUIDATION_MAX_SLIP_BPS ({}) must be <= 10000",
                max_slip_bps
            ),
        ));
    }

    Ok(ShardConfig {
        shard_id,
        shard_count,
        max_symbols,
        symbol_params: vec![None; max_symbols],
        taker_fee_bps: vec![0; max_symbols],
        maker_fee_bps: vec![0; max_symbols],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig {
            base_delay_ns: env_parse("RSX_LIQUIDATION_BASE_DELAY_NS", 100_000_000u64)?,
            socialized_cooldown_ns: env_parse(
                "RSX_LIQUIDATION_SOCIALIZED_COOLDOWN_NS",
                120_000_000_000u64,
            )?,
            base_slip_bps: env_parse("RSX_LIQUIDATION_BASE_SLIP_BPS", 1u64)?,
            max_rounds: env_parse("RSX_LIQUIDATION_MAX_ROUNDS", 10u32)?,
            max_slip_bps,
        },
        replication_config: ReplicationConfig {
            lease_poll_interval_ms: env_parse("RSX_RISK_LEASE_POLL_MS", 500u64)?,
            lease_renew_interval_ms: env_parse("RSX_RISK_LEASE_RENEW_MS", 1000u64)?,
        },
    })
}

const BASE_ME_CAST: u16 = 9100;

/// Parse a comma-separated ME cast address string into a
/// symbol_id → SocketAddr map. symbol_id = port - BASE_ME_CAST.
pub fn parse_me_cast_addrs(raw: &str) -> HashMap<u32, SocketAddr> {
    let mut map = HashMap::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<SocketAddr>() {
            Ok(addr) => {
                let sid = addr.port().saturating_sub(BASE_ME_CAST) as u32;
                map.insert(sid, addr);
            }
            Err(e) => {
                warn!("skipping invalid ME addr '{}': {}", part, e);
            }
        }
    }
    map
}

/// Read ME cast addresses from env. Prefers `RSX_ME_CAST_ADDRS`
/// (comma-separated), falls back to `RSX_ME_CAST_ADDR` (single),
/// then defaults to `127.0.0.1:9110`.
pub fn me_cast_addrs_from_env() -> HashMap<u32, SocketAddr> {
    let raw = std::env::var("RSX_ME_CAST_ADDRS")
        .or_else(|_| std::env::var("RSX_ME_CAST_ADDR"))
        .unwrap_or_else(|_| "127.0.0.1:9110".to_owned());
    parse_me_cast_addrs(&raw)
}

fn parse_symbol_addrs(key: &str, raw: &str) -> io::Result<HashMap<u32, SocketAddr>> {
    let mut addrs = HashMap::new();
    for entry in raw.split(',') {
        let (symbol, addr) = entry.trim().split_once('=').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid {key} entry: {entry}"),
            )
        })?;
        let symbol_id = symbol.parse::<u32>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid {key} symbol: {symbol}"),
            )
        })?;
        let addr = addr.parse::<SocketAddr>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid {key} address: {addr}"),
            )
        })?;
        if addrs.insert(symbol_id, addr).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("duplicate symbol {symbol_id} in {key}"),
            ));
        }
    }
    Ok(addrs)
}

pub fn parse_me_streams(
    cast_addrs: HashMap<u32, SocketAddr>,
    recv_addrs: Option<&str>,
    replay_addrs: Option<&str>,
    singular_recv: Option<SocketAddr>,
    singular_replay: Option<&str>,
) -> io::Result<Vec<MeStreamConfig>> {
    if cast_addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no ME cast addresses configured",
        ));
    }
    if cast_addrs.len() == 1 && recv_addrs.is_none() && replay_addrs.is_none() {
        let (&symbol_id, &cast_addr) = cast_addrs.iter().next().expect("non-empty map");
        let recv_addr = singular_recv.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "RSX_RISK_ME_RECV_ADDR required",
            )
        })?;
        let replay_addr = singular_replay.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "RSX_ME_REPLICATION_ADDR required",
            )
        })?;
        return Ok(vec![MeStreamConfig {
            symbol_id,
            cast_addr,
            recv_addr,
            replay_addr: replay_addr.to_owned(),
        }]);
    }
    let recv_addrs = recv_addrs.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "RSX_RISK_ME_RECV_ADDRS required for multiple MEs",
        )
    })?;
    let replay_addrs = replay_addrs.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "RSX_ME_REPLICATION_ADDRS required for multiple MEs",
        )
    })?;
    let recv = parse_symbol_addrs("RSX_RISK_ME_RECV_ADDRS", recv_addrs)?;
    let replay = parse_symbol_addrs("RSX_ME_REPLICATION_ADDRS", replay_addrs)?;
    if recv.len() != cast_addrs.len() || replay.len() != cast_addrs.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ME cast, receive, and replay symbol sets must match",
        ));
    }
    let mut used_recv = std::collections::HashSet::new();
    let mut streams = Vec::with_capacity(cast_addrs.len());
    for (symbol_id, cast_addr) in cast_addrs {
        let recv_addr = *recv.get(&symbol_id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("missing receive address for symbol {symbol_id}"),
            )
        })?;
        let replay_addr = replay.get(&symbol_id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("missing replay address for symbol {symbol_id}"),
            )
        })?;
        if !used_recv.insert(recv_addr) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("duplicate ME receive bind {recv_addr}"),
            ));
        }
        streams.push(MeStreamConfig {
            symbol_id,
            cast_addr,
            recv_addr,
            replay_addr: replay_addr.to_string(),
        });
    }
    streams.sort_by_key(|stream| stream.symbol_id);
    Ok(streams)
}

/// Runtime env config for the risk process, parsed ONCE at boot
/// with fail-fast on malformed required values. Distinct from
/// `ShardConfig` (sharding/margin/liquidation params) — this
/// holds the process's I/O topology (PG url, WAL dir, cast
/// addrs). Defaults and required-vs-optional match the original
/// scattered `env::var` sites in `run_main` exactly.
pub struct RuntimeConfig {
    /// Postgres connection string. Required.
    pub db_url: String,
    /// WAL directory. Default `./tmp/wal`.
    pub wal_dir: PathBuf,
    /// Per-symbol live receive and replay topology.
    pub me_streams: Vec<MeStreamConfig>,
    /// Mark cast receive addr. Default `127.0.0.1:9105`.
    pub mark_addr: SocketAddr,
    /// Mark cast sender (peer) addr. Default `127.0.0.1:9106`.
    pub mark_sender_addr: SocketAddr,
    /// Mark WAL replication endpoint for FAULTED recovery. `None`
    /// skips the gap (mark is latest-wins state).
    pub mark_replay_addr: Option<String>,
    /// Risk cast receive addr (from gateway). Default
    /// `127.0.0.1:9101`.
    pub risk_addr: SocketAddr,
    /// Gateway cast addr (responses out). Default
    /// `127.0.0.1:9102`.
    pub gw_addr: SocketAddr,
    /// Gateway WAL replication endpoint for FAULTED recovery. `None`
    /// skips the gap (pre-ack orders are client-resent, ME-deduped).
    pub gw_replay_addr: Option<String>,
    /// Optional bind addr hint for the ME-bound sender.
    pub me_send_bind: Option<String>,
    /// Hot-thread core to pin; `None` leaves the thread unpinned.
    pub core_id: Option<usize>,
}

impl RuntimeConfig {
    /// Parse the runtime config from env. Required values
    /// (`DATABASE_URL`, ME replay topology) and malformed
    /// SocketAddrs fail-fast via `expect` — a bad addr must fail
    /// loudly at boot, not silently default mid-run.
    pub fn from_env() -> Self {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required for risk");
        let wal_dir =
            PathBuf::from(std::env::var("RSX_RISK_WAL_DIR").unwrap_or_else(|_| "./tmp/wal".into()));
        let me_streams = parse_me_streams(
            me_cast_addrs_from_env(),
            std::env::var("RSX_RISK_ME_RECV_ADDRS").ok().as_deref(),
            std::env::var("RSX_ME_REPLICATION_ADDRS").ok().as_deref(),
            std::env::var("RSX_RISK_ME_RECV_ADDR")
                .ok()
                .map(|value| value.parse().expect("invalid RSX_RISK_ME_RECV_ADDR"))
                .or_else(|| Some("127.0.0.1:28301".parse().expect("valid default"))),
            std::env::var("RSX_ME_REPLICATION_ADDR").ok().as_deref(),
        )
        .expect("invalid ME stream configuration");
        let mark_addr = parse_addr("RSX_RISK_MARK_CAST_ADDR", "127.0.0.1:9105");
        let mark_sender_addr = parse_addr("RSX_MARK_CAST_ADDR", "127.0.0.1:9106");
        let mark_replay_addr = std::env::var("RSX_MARK_REPLICATION_ADDR").ok();
        let risk_addr = parse_addr("RSX_RISK_CAST_ADDR", "127.0.0.1:9101");
        let gw_addr = parse_addr("RSX_GW_CAST_ADDR", "127.0.0.1:9102");
        let gw_replay_addr = std::env::var("RSX_GW_REPLICATION_ADDR").ok();
        let me_send_bind = std::env::var("RSX_RISK_ME_SEND_ADDR").ok();
        let core_id = std::env::var("RSX_RISK_CORE_ID")
            .ok()
            .map(|raw| raw.parse().expect("invalid RSX_RISK_CORE_ID"));

        Self {
            db_url,
            wal_dir,
            me_streams,
            mark_addr,
            mark_sender_addr,
            mark_replay_addr,
            risk_addr,
            gw_addr,
            gw_replay_addr,
            me_send_bind,
            core_id,
        }
    }
}

/// Parse a SocketAddr from `key`, defaulting to `default` when
/// unset. A malformed value (env or default) fails fast.
fn parse_addr(key: &str, default: &str) -> SocketAddr {
    std::env::var(key)
        .unwrap_or_else(|_| default.into())
        .parse()
        .unwrap_or_else(|_| panic!("invalid {key}"))
}

/// One active row of `symbol_config_schedule`, the part risk
/// enforces: margin rates, leverage cap and fees, stamped with the
/// version it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolRiskConfig {
    pub symbol_id: u32,
    pub config_version: u64,
    pub params: SymbolRiskParams,
    pub taker_fee_bps: i64,
    pub maker_fee_bps: i64,
}

const SYMBOL_CONFIG_COLUMNS: &str = "symbol_id, config_version, \
     initial_margin_rate_bps, maintenance_margin_rate_bps, max_leverage, \
     taker_fee_bps, maker_fee_bps";

/// The active config of every symbol below `max_symbols`: per
/// symbol, the highest `config_version` whose `effective_at_ms` is
/// at or before `now_ms`. Symbols without a row are absent.
pub async fn load_symbol_configs(
    client: &Client,
    max_symbols: usize,
    now_ms: u64,
) -> io::Result<Vec<SymbolRiskConfig>> {
    let rows = client
        .query(
            &format!(
                "SELECT DISTINCT ON (symbol_id) {SYMBOL_CONFIG_COLUMNS} \
                 FROM symbol_config_schedule \
                 WHERE symbol_id < $1 AND effective_at_ms <= $2 \
                 ORDER BY symbol_id, config_version DESC"
            ),
            &[&(max_symbols as i32), &(now_ms as i64)],
        )
        .await
        .map_err(|e| io::Error::other(format!("load_symbol_configs: {e}")))?;
    rows.iter().map(symbol_config_from_row).collect()
}

/// Exactly one scheduled version of one symbol, as named by a
/// `CONFIG_APPLIED` record. `None` when no such row exists.
pub async fn load_symbol_config(
    client: &Client,
    symbol_id: u32,
    config_version: u64,
) -> io::Result<Option<SymbolRiskConfig>> {
    let row = client
        .query_opt(
            &format!(
                "SELECT {SYMBOL_CONFIG_COLUMNS} FROM symbol_config_schedule \
                 WHERE symbol_id = $1 AND config_version = $2"
            ),
            &[&(symbol_id as i32), &(config_version as i64)],
        )
        .await
        .map_err(|e| io::Error::other(format!("load_symbol_config: {e}")))?;
    row.as_ref().map(symbol_config_from_row).transpose()
}

fn symbol_config_from_row(row: &tokio_postgres::Row) -> io::Result<SymbolRiskConfig> {
    let symbol_id = row.get::<_, i32>(0) as u32;
    let config_version = row.get::<_, i64>(1) as u64;
    let column = |idx: usize, name: &str| -> io::Result<i64> {
        let value: Option<i32> = row.get(idx);
        value.map(i64::from).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("symbol {symbol_id} v{config_version}: {name} is NULL"),
            )
        })
    };
    Ok(SymbolRiskConfig {
        symbol_id,
        config_version,
        params: SymbolRiskParams {
            initial_margin_rate: column(2, "initial_margin_rate_bps")?,
            maintenance_margin_rate: column(3, "maintenance_margin_rate_bps")?,
            max_leverage: column(4, "max_leverage")?,
        },
        taker_fee_bps: column(5, "taker_fee_bps")?,
        maker_fee_bps: column(6, "maker_fee_bps")?,
    })
}

impl RiskShard {
    /// Install one symbol's risk config: margin rates, leverage cap,
    /// fees and the version they came from. The one write path for
    /// symbol config, at boot, at promotion and on live reload.
    pub fn apply_symbol_config(&mut self, config: &SymbolRiskConfig) {
        let sid = config.symbol_id as usize;
        if sid >= self.config_versions.len() {
            warn!(
                "symbol config for symbol {} ignored: max_symbols={}",
                config.symbol_id,
                self.config_versions.len()
            );
            return;
        }
        self.margin.symbol_params[sid] = Some(config.params.clone());
        self.taker_fee_bps[sid] = config.taker_fee_bps;
        self.maker_fee_bps[sid] = config.maker_fee_bps;
        self.config_versions[sid] = config.config_version;
        info!(
            "risk symbol_config sid={} v={} im_rate={} mm_rate={} max_leverage={} taker_fee_bps={} maker_fee_bps={}",
            config.symbol_id,
            config.config_version,
            config.params.initial_margin_rate,
            config.params.maintenance_margin_rate,
            config.params.max_leverage,
            config.taker_fee_bps,
            config.maker_fee_bps,
        );
    }

    /// A `CONFIG_APPLIED` record from the ME: ask the persist worker
    /// to fetch that version (the pinned loop never touches
    /// Postgres). Versions at or below the applied one are ignored.
    pub fn process_config_applied(&mut self, symbol_id: u32, config_version: u64) {
        let sid = symbol_id as usize;
        if sid >= self.config_versions.len() {
            return;
        }
        if config_version <= self.config_versions[sid] {
            return;
        }
        self.push_persist(PersistEvent::ConfigReload {
            symbol_id,
            config_version,
        });
    }

    pub(crate) fn drain_config_updates(&mut self) {
        let Some(mut updates) = self.config_updates.take() else {
            return;
        };
        while let Ok(config) = updates.pop() {
            self.apply_symbol_config(&config);
        }
        self.config_updates = Some(updates);
    }
}

/// Log the effective shard config at boot: sharding, lease and
/// liquidation knobs and the routing rule with worked examples.
/// Symbol params are logged as each is applied.
pub fn log_shard_config(config: &ShardConfig) {
    info!(
        "risk effective config: shard_id={} shard_count={} max_symbols={} lease_poll_ms={} lease_renew_ms={} liquidation_base_delay_ns={} liquidation_base_slip_bps={} liquidation_max_rounds={}",
        config.shard_id,
        config.shard_count,
        config.max_symbols,
        config.replication_config.lease_poll_interval_ms,
        config.replication_config.lease_renew_interval_ms,
        config.liquidation_config.base_delay_ns,
        config.liquidation_config.base_slip_bps,
        config.liquidation_config.max_rounds,
    );
    info!(
        "risk shard_routing rule='user_id % shard_count == shard_id' shard_id={} shard_count={}",
        config.shard_id, config.shard_count,
    );
    for user_id in 0u32..8 {
        let owner = user_id % config.shard_count;
        let serves = owner == config.shard_id;
        info!(
            "risk shard_routing_example user_id={} owner_shard={} served_here={}",
            user_id, owner, serves
        );
    }
}
