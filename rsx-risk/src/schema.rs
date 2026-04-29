use tokio_postgres::Client;
use tokio_postgres::Error;

const MIGRATION_001: &str =
    include_str!("../migrations/001_base_schema.sql");
const MIGRATION_002: &str =
    include_str!("../migrations/002_rename_tables.sql");
const MIGRATION_003: &str =
    include_str!("../migrations/003_users.sql");
const MIGRATION_004: &str =
    include_str!("../migrations/004_frozen_orders.sql");

pub async fn run_migrations(
    client: &Client,
) -> Result<(), Error> {
    client.batch_execute(MIGRATION_001).await?;
    client.batch_execute(MIGRATION_002).await?;
    client.batch_execute(MIGRATION_003).await?;
    client.batch_execute(MIGRATION_004).await?;
    Ok(())
}
