use crate::funding::FundingConfig;
use crate::margin::SymbolRiskParams;
use std::io;

pub struct LiquidationConfig {
    pub base_delay_ns: u64,
    pub base_slip_bps: u64,
    pub max_rounds: u32,
}

impl Default for LiquidationConfig {
    fn default() -> Self {
        Self {
            base_delay_ns: 100_000_000, // 100ms
            base_slip_bps: 1,
            max_rounds: 10,
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

pub fn load_shard_config() -> io::Result<ShardConfig> {
    let shard_id = env_u32("RSX_RISK_SHARD_ID", 0);
    let shard_count = env_u32("RSX_RISK_SHARD_COUNT", 1);
    let max_symbols =
        env_usize("RSX_RISK_MAX_SYMBOLS", 64);

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

    Ok(ShardConfig {
        shard_id,
        shard_count,
        max_symbols,
        symbol_params,
        taker_fee_bps,
        maker_fee_bps,
        funding_config: FundingConfig::default(),
        liquidation_config:
            LiquidationConfig::default(),
    })
}
