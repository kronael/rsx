use rsx_types::SymbolConfig;
use std::io;
use tokio_postgres::Client;
use tracing::info;

/// Scheduled config from the database
#[derive(Debug, Clone)]
pub struct ScheduledConfig {
    pub config_version: u64,
    pub effective_at_ms: u64,
    pub tick_size: i64,
    pub lot_size: i64,
    pub price_decimals: u8,
    pub qty_decimals: u8,
}

impl ScheduledConfig {
    pub fn to_symbol_config(
        &self,
        symbol_id: u32,
    ) -> SymbolConfig {
        SymbolConfig {
            symbol_id,
            tick_size: self.tick_size,
            lot_size: self.lot_size,
            price_decimals: self.price_decimals,
            qty_decimals: self.qty_decimals,
        }
    }
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
            &[&(symbol_id as i32), &(current_version as i64), &(now_ms as i64)],
        )
        .await
        .map_err(|e| {
            io::Error::other(
                format!("poll_scheduled_configs: {}", e),
            )
        })?;

    let mut configs = Vec::new();
    for row in rows {
        let config = ScheduledConfig {
            config_version: row.get::<_, i64>(0) as u64,
            effective_at_ms: row.get::<_, i64>(1) as u64,
            tick_size: row.get(2),
            lot_size: row.get(3),
            price_decimals: row.get::<_, i16>(4) as u8,
            qty_decimals: row.get::<_, i16>(5) as u8,
        };
        configs.push(config);
    }
    Ok(configs)
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
                &config.tick_size,
                &config.lot_size,
                &(config.price_decimals as i16),
                &(config.qty_decimals as i16),
            ],
        )
        .await
        .map_err(|e| {
            io::Error::other(
                format!("write_applied_config: {}", e),
            )
        })?;

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
        .await
        .map_err(|e| {
            io::Error::other(
                format!("load_applied_config: {}", e),
            )
        })?;

    Ok(row.map(|r| ScheduledConfig {
        config_version: r.get::<_, i64>(0) as u64,
        effective_at_ms: r.get::<_, i64>(1) as u64,
        tick_size: r.get(2),
        lot_size: r.get(3),
        price_decimals: r.get::<_, i16>(4) as u8,
        qty_decimals: r.get::<_, i16>(5) as u8,
    }))
}
