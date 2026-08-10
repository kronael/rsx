use tokio_postgres::Client;
use tokio_postgres::Error;

const MIGRATION_001: &str = include_str!("../migrations/001_symbol_config.sql");
const MIGRATION_002: &str = include_str!("../migrations/002_seed_symbols.sql");

/// Apply the symbol-config migrations in order. Each file is
/// version-guarded against the shared `migrations(id)` table and
/// idempotent, so every ME can run this at boot concurrently.
pub async fn run_migrations(client: &Client) -> Result<(), Error> {
    client.batch_execute(MIGRATION_001).await?;
    client.batch_execute(MIGRATION_002).await?;
    Ok(())
}
