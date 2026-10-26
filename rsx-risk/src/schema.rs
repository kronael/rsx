use tokio_postgres::Client;
use tokio_postgres::Error;

const MIGRATION_001: &str = include_str!("../migrations/001_base_schema.sql");
const MIGRATION_002: &str = include_str!("../migrations/002_rename_tables.sql");
const MIGRATION_003: &str = include_str!("../migrations/003_users.sql");
const MIGRATION_004: &str = include_str!("../migrations/004_frozen_orders.sql");
// Symbol catalog + config schedule. Risk owns the database, so risk
// applies these; the ME and the dashboard are readers. The ids inside
// the files stay 001_symbol_config/002_seed_symbols -- already recorded
// in deployed `migrations` tables, and their DDL is not re-runnable.
const MIGRATION_005: &str = include_str!("../migrations/005_symbol_config.sql");
const MIGRATION_006: &str = include_str!("../migrations/006_seed_symbols.sql");
const MIGRATION_007: &str = include_str!("../migrations/007_min_order_notional.sql");

pub async fn run_migrations(client: &Client) -> Result<(), Error> {
    client.batch_execute(MIGRATION_001).await?;
    client.batch_execute(MIGRATION_002).await?;
    client.batch_execute(MIGRATION_003).await?;
    client.batch_execute(MIGRATION_004).await?;
    client.batch_execute(MIGRATION_005).await?;
    client.batch_execute(MIGRATION_006).await?;
    client.batch_execute(MIGRATION_007).await?;
    Ok(())
}
