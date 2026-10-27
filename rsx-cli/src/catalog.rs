use crate::output::Failure;
use crate::plan::CatalogSymbol;
use rsx_risk::db::connect;
use rsx_risk::schema::run_migrations;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[cfg(test)]
#[path = "catalog_test.rs"]
mod catalog_test;

const CATALOG_SQL: &str = "SELECT DISTINCT ON (symbol_static.symbol_id) \
     symbol_static.symbol_id, symbol_static.symbol_name, \
     symbol_config_schedule.tick_size, symbol_config_schedule.lot_size, \
     symbol_config_schedule.price_decimals, symbol_config_schedule.qty_decimals \
     FROM symbol_static JOIN symbol_config_schedule \
     ON symbol_config_schedule.symbol_id = symbol_static.symbol_id \
     WHERE symbol_config_schedule.effective_at_ms <= $1 \
     ORDER BY symbol_static.symbol_id, symbol_config_schedule.config_version DESC";

/// `url` with the password removed, safe to print: the `user:password@` of a
/// URL, or the `password=` word of a key-value connection string.
pub fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url
            .split_whitespace()
            .map(|word| {
                if word.starts_with("password=") {
                    "password=***"
                } else {
                    word
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    };
    let Some((credentials, host)) = rest.rsplit_once('@') else {
        return url.to_string();
    };
    let user = credentials.split(':').next().unwrap_or(credentials);
    format!("{scheme}://{user}@{host}")
}

/// Every symbol with an active config at `database_url`, ordered by id.
/// Applies risk's migrations first (risk applies the same ones at boot), so
/// a fresh database holds the seeded symbols. An empty `symbol_static` is a
/// `Runtime` failure.
pub async fn load(database_url: &str) -> Result<Vec<CatalogSymbol>, Failure> {
    let shown = redact(database_url);
    let client = connect(database_url).await.map_err(|error| {
        Failure::runtime(format!(
            "postgres is not reachable at {shown}: {error} (start it, or pass --db)"
        ))
    })?;
    run_migrations(&client)
        .await
        .map_err(|error| Failure::runtime(format!("migrations failed at {shown}: {error}")))?;
    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM symbol_static", &[])
        .await
        .and_then(|row| row.try_get(0))
        .map_err(|error| Failure::runtime(format!("failed to read symbol_static: {error}")))?;
    if count == 0 {
        return Err(Failure::runtime(format!(
            "symbol_static is empty at {shown}: the cluster takes its symbols from risk's \
             Postgres (insert rows as rsx-risk/migrations/006_seed_symbols.sql does)"
        )));
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64);
    let rows = client
        .query(CATALOG_SQL, &[&now_ms])
        .await
        .map_err(|error| Failure::runtime(format!("failed to read the symbol config: {error}")))?;
    let mut symbols = Vec::new();
    for row in &rows {
        let read = |error: tokio_postgres::Error| {
            Failure::runtime(format!("bad symbol config row: {error}"))
        };
        symbols.push(CatalogSymbol {
            symbol_id: row.try_get::<_, i32>(0).map_err(read)? as u32,
            name: row.try_get(1).map_err(read)?,
            tick_size: row.try_get(2).map_err(read)?,
            lot_size: row.try_get(3).map_err(read)?,
            price_decimals: row.try_get::<_, i16>(4).map_err(read)? as u32,
            qty_decimals: row.try_get::<_, i16>(5).map_err(read)? as u32,
        });
    }
    Ok(symbols)
}
