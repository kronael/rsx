use tokio_postgres::Client;
use tokio_postgres::Error;

pub const MIGRATION_001: &str =
    include_str!("../migrations/001_base_schema.sql");

pub async fn run_migrations(
    client: &Client,
) -> Result<(), Error> {
    client.batch_execute(MIGRATION_001).await
}
