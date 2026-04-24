use crate::funding::FundingConfig;
use crate::margin::SymbolRiskParams;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use tracing::warn;

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
    pub is_replica: bool,
    pub lease_poll_interval_ms: u64,
    pub lease_renew_interval_ms: u64,
    pub replica_sync_ring_size: usize,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            is_replica: false,
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
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

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn load_shard_config() -> io::Result<ShardConfig> {
    let shard_id = env_u32("RSX_RISK_SHARD_ID", 0);
    let shard_count = env_u32("RSX_RISK_SHARD_COUNT", 1);
    let max_symbols =
        env_usize("RSX_RISK_MAX_SYMBOLS", 64);
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
            initial_margin_rate: 1000, // 10%
            maintenance_margin_rate: 500, // 5%
            max_leverage: 10,
        });
        taker_fee_bps.push(5); // 0.05%
        maker_fee_bps.push(-1); // -0.01% rebate
    }

    let is_replica =
        env_bool("RSX_RISK_IS_REPLICA", false);
    let lease_poll_interval_ms =
        env_u64("RSX_RISK_LEASE_POLL_MS", 500);
    let lease_renew_interval_ms =
        env_u64("RSX_RISK_LEASE_RENEW_MS", 1000);

    Ok(ShardConfig {
        shard_id,
        shard_count,
        max_symbols,
        symbol_params,
        taker_fee_bps,
        maker_fee_bps,
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig {
            base_delay_ns: env_u64(
                "RSX_LIQUIDATION_BASE_DELAY_NS",
                100_000_000,
            ),
            base_slip_bps: env_u64(
                "RSX_LIQUIDATION_BASE_SLIP_BPS", 1),
            max_rounds: env_u64(
                "RSX_LIQUIDATION_MAX_ROUNDS", 10) as u32,
            max_slip_bps: env_u64(
                "RSX_LIQUIDATION_MAX_SLIP_BPS", 9999),
        },
        replication_config: ReplicationConfig {
            is_replica,
            lease_poll_interval_ms,
            lease_renew_interval_ms,
            replica_sync_ring_size: 1024,
        },
    })
}

const BASE_ME_CMP: u16 = 9100;

/// Parse a comma-separated ME CMP address string into a
/// symbol_id → SocketAddr map. symbol_id = port - BASE_ME_CMP.
pub fn parse_me_cmp_addrs(raw: &str) -> HashMap<u32, SocketAddr> {
    let mut map = HashMap::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<SocketAddr>() {
            Ok(addr) => {
                let sid =
                    addr.port().saturating_sub(BASE_ME_CMP) as u32;
                map.insert(sid, addr);
            }
            Err(e) => {
                warn!(
                    "skipping invalid ME addr '{}': {}",
                    part, e
                );
            }
        }
    }
    map
}

/// Read ME CMP addresses from env. Prefers `RSX_ME_CMP_ADDRS`
/// (comma-separated), falls back to `RSX_ME_CMP_ADDR` (single),
/// then defaults to `127.0.0.1:9110`.
pub fn me_cmp_addrs_from_env() -> HashMap<u32, SocketAddr> {
    let raw = std::env::var("RSX_ME_CMP_ADDRS")
        .or_else(|_| std::env::var("RSX_ME_CMP_ADDR"))
        .unwrap_or_else(|_| "127.0.0.1:9110".to_owned());
    parse_me_cmp_addrs(&raw)
}
