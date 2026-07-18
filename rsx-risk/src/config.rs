use crate::funding::FundingConfig;
use crate::margin::SymbolRiskParams;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
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
    /// Cap on aggregate slippage (round^2 * base_slip_bps)
    /// applied when generating liquidation orders. Default
    /// 9999 (99.99%) — loose cap preserving legacy behavior.
    /// Lower values halt runaway slippage sooner.
    pub max_slip_bps: u64,
}

impl Default for LiquidationConfig {
    fn default() -> Self {
        Self {
            base_delay_ns: 100_000_000, // 100ms
            base_slip_bps: 1,
            max_rounds: 10,
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

pub struct ShardConfig {
    pub shard_id: u32,
    pub shard_count: u32,
    pub max_symbols: usize,
    pub symbol_params: Vec<SymbolRiskParams>,
    pub taker_fee_bps: Vec<i64>,
    pub maker_fee_bps: Vec<i64>,
    pub funding_config: FundingConfig,
    pub liquidation_config: LiquidationConfig,
    pub replication_config: ReplicationConfig,
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn load_shard_config() -> io::Result<ShardConfig> {
    let shard_id = env_u32("RSX_RISK_SHARD_ID", 0);
    let shard_count = env_u32("RSX_RISK_SHARD_COUNT", 1);
    let max_symbols = env_usize("RSX_RISK_MAX_SYMBOLS", 64);
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

    let mut symbol_params = Vec::with_capacity(max_symbols);
    let mut taker_fee_bps = Vec::with_capacity(max_symbols);
    let mut maker_fee_bps = Vec::with_capacity(max_symbols);
    for _ in 0..max_symbols {
        symbol_params.push(SymbolRiskParams {
            initial_margin_rate: 1000,    // 10%
            maintenance_margin_rate: 500, // 5%
            max_leverage: 10,
        });
        taker_fee_bps.push(5); // 0.05%
        maker_fee_bps.push(-1); // -0.01% rebate
    }

    // Long-side liq price is mark*(10000-slip)/10000; slip
    // above 10_000 bps drives it negative. Fail fast.
    let max_slip_bps = env_u64("RSX_LIQUIDATION_MAX_SLIP_BPS", 9999);
    if max_slip_bps > 10_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "RSX_LIQUIDATION_MAX_SLIP_BPS ({}) must be <= 10000",
                max_slip_bps
            ),
        ));
    }

    let lease_poll_interval_ms = env_u64("RSX_RISK_LEASE_POLL_MS", 500);
    let lease_renew_interval_ms = env_u64("RSX_RISK_LEASE_RENEW_MS", 1000);

    Ok(ShardConfig {
        shard_id,
        shard_count,
        max_symbols,
        symbol_params,
        taker_fee_bps,
        maker_fee_bps,
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig {
            base_delay_ns: env_u64("RSX_LIQUIDATION_BASE_DELAY_NS", 100_000_000),
            base_slip_bps: env_u64("RSX_LIQUIDATION_BASE_SLIP_BPS", 1),
            max_rounds: env_u64("RSX_LIQUIDATION_MAX_ROUNDS", 10) as u32,
            max_slip_bps,
        },
        replication_config: ReplicationConfig {
            lease_poll_interval_ms,
            lease_renew_interval_ms,
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
    /// Risk cast receive addr (from gateway). Default
    /// `127.0.0.1:9101`.
    pub risk_addr: SocketAddr,
    /// Gateway cast addr (responses out). Default
    /// `127.0.0.1:9102`.
    pub gw_addr: SocketAddr,
    /// Optional bind addr hint for the ME-bound sender.
    pub me_send_bind: Option<String>,
    /// Optional hot-thread core to pin. `None` if unset OR
    /// unparseable (silent skip preserves legacy behavior — a
    /// bad core id is NOT a boot failure).
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
        let risk_addr = parse_addr("RSX_RISK_CAST_ADDR", "127.0.0.1:9101");
        let gw_addr = parse_addr("RSX_GW_CAST_ADDR", "127.0.0.1:9102");
        let me_send_bind = std::env::var("RSX_RISK_ME_SEND_ADDR").ok();
        // Silent skip on unset OR unparseable (legacy: a bad
        // core id is a no-op, not a boot failure).
        let core_id = std::env::var("RSX_RISK_CORE_ID")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());

        Self {
            db_url,
            wal_dir,
            me_streams,
            mark_addr,
            mark_sender_addr,
            risk_addr,
            gw_addr,
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
        // SAFETY: fail-fast at startup
        .unwrap_or_else(|_| panic!("invalid {key}"))
}
