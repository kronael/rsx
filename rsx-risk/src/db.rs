//! Postgres connect helper for the risk process.
//!
//! `tokio_postgres::connect` hands back the `Client` and a
//! `Connection` future separately; the future must be polled on a
//! task for the client to make progress. `connect` bundles the
//! connect + spawn so callers get a ready-to-use `Client`.

use tracing::error;

/// Drive a tokio_postgres connection to completion. Named (not an
/// inline `tokio::spawn(async move {…})`) per CLAUDE.md so the
/// coroutine's lifetime is visible to the reader.
pub async fn drive_pg_connection(
    connection: tokio_postgres::Connection<
        tokio_postgres::Socket,
        tokio_postgres::tls::NoTlsStream,
    >,
) {
    if let Err(e) = connection.await {
        error!("pg connection error: {e}");
    }
}

/// Connect to Postgres and spawn the connection driver task.
/// Must be called from within a tokio runtime (the driver is
/// spawned via `tokio::spawn`).
pub async fn connect(url: &str) -> Result<tokio_postgres::Client, tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    tokio::spawn(drive_pg_connection(connection));
    Ok(client)
}
