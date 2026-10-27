use crate::catalog::redact;
use crate::output::Failure;
use rsx_risk::db::connect;
use rsx_risk::schema::run_migrations;
use tokio_postgres::Client;
use tokio_postgres::Config;

#[cfg(test)]
#[path = "database_test.rs"]
mod database_test;

/// Fails fast on a url Postgres cannot parse, before any connection: a
/// typo in `DATABASE_URL` is `Misuse`, never a connect error. The message
/// leaves the url out, since it may carry a password.
pub fn check_url(url: &str) -> Result<(), Failure> {
    url.parse::<Config>().map(|_| ()).map_err(|error| {
        Failure::misuse(format!(
            "DATABASE_URL (or --db) is not a valid postgres url: {error}"
        ))
    })
}

/// A client on risk's Postgres with every migration applied, as the daemons
/// have it at boot. An unparsable `url` is `Misuse`; an unreachable server
/// is `Runtime`.
pub async fn open(url: &str) -> Result<Client, Failure> {
    check_url(url)?;
    let shown = redact(url);
    let client = connect(url).await.map_err(|error| {
        Failure::runtime(format!(
            "postgres is not reachable at {shown}: {error} (start it, or pass --db)"
        ))
    })?;
    run_migrations(&client)
        .await
        .map_err(|error| Failure::runtime(format!("migrations failed at {shown}: {error}")))?;
    Ok(client)
}
