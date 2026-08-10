//! Shared Postgres testcontainers setup for this crate's integration tests.
//!
//! Mirrored in `rsx-matching/tests/common/mod.rs` — keep the two in sync.
#![allow(dead_code)] // each test binary compiles this module and uses a subset

use std::time::Duration;
use std::time::Instant;

use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::Image;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::Client;
use tokio_postgres::NoTls;

const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// `Postgres` image with connection-probe readiness instead of the default
/// log-line wait: a Docker daemon configured with `LoggingDriver=none`
/// answers log reads with HTTP 501, so a log-based `WaitFor` can never
/// succeed there. `start_postgres` polls real connections instead.
#[derive(Debug, Default)]
pub struct PostgresNoLogWait(Postgres);

impl Image for PostgresNoLogWait {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn tag(&self) -> &str {
        self.0.tag()
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        Vec::new()
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<
        Item = (
            impl Into<std::borrow::Cow<'_, str>>,
            impl Into<std::borrow::Cow<'_, str>>,
        ),
    > {
        self.0.env_vars()
    }
}

/// Start a Postgres container and block until it accepts real connections.
/// Returns the container handle (keep it alive for the test's duration)
/// and the mapped host port.
pub async fn start_postgres() -> (ContainerAsync<PostgresNoLogWait>, u16) {
    let container = PostgresNoLogWait::default()
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("mapped port for 5432");
    wait_until_ready(port).await;
    (container, port)
}

/// Poll until Postgres completes a full connection handshake. The official
/// entrypoint's init phase listens on a unix socket only, so a successful
/// TCP handshake proves the final server is up. Also used to re-await
/// readiness after a container stop/start cycle.
pub async fn wait_until_ready(port: u16) {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        match tokio_postgres::connect(&connstr(port), NoTls).await {
            Ok(_) => return,
            Err(e) => {
                if Instant::now() >= deadline {
                    panic!("postgres on port {port} not ready within {READY_TIMEOUT:?}: {e}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

pub fn connstr(port: u16) -> String {
    format!(
        "host=127.0.0.1 port={port} user=postgres \
         password=postgres dbname=postgres"
    )
}

/// Open a fresh client on the given port; the connection driver runs in a
/// named background task.
pub async fn connect(port: u16) -> Client {
    let (client, conn) = tokio_postgres::connect(&connstr(port), NoTls)
        .await
        .expect("connect to postgres");
    tokio::spawn(drive_connection(conn));
    client
}

async fn drive_connection(
    conn: tokio_postgres::Connection<tokio_postgres::Socket, tokio_postgres::tls::NoTlsStream>,
) {
    if let Err(e) = conn.await {
        eprintln!("pg conn closed: {e}");
    }
}
