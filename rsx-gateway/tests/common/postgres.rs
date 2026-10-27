//! A Postgres testcontainer with risk's symbol migrations applied, the
//! config the gateway reads. Mirrors `rsx-cli/tests/common/postgres.rs`
//! (container, readiness probe) and applies the migration files the way
//! `rsx-matching/tests/config_poll_test.rs` does.
#![allow(dead_code)] // each test binary compiles this module and uses a subset

use std::time::Duration;
use std::time::Instant;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::Image;
use testcontainers_modules::postgres::Postgres;
use tokio::runtime::Runtime;
use tokio_postgres::Client;
use tokio_postgres::NoTls;

const READY_TIMEOUT: Duration = Duration::from_secs(60);

const SYMBOL_MIGRATIONS: [&str; 3] = [
    include_str!("../../../rsx-risk/migrations/005_symbol_config.sql"),
    include_str!("../../../rsx-risk/migrations/006_seed_symbols.sql"),
    include_str!("../../../rsx-risk/migrations/007_min_order_notional.sql"),
];

/// `Postgres` image with connection-probe readiness: a Docker daemon with
/// `LoggingDriver=none` answers log reads with HTTP 501, so the default
/// log-line wait never succeeds there.
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

pub struct TestPostgres {
    container: Option<ContainerAsync<PostgresNoLogWait>>,
    pub runtime: Runtime,
    pub url: String,
    pub client: Client,
}

impl Drop for TestPostgres {
    fn drop(&mut self) {
        let _entered = self.runtime.enter();
        drop(self.container.take());
    }
}

impl TestPostgres {
    /// A running Postgres with no schema.
    pub fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (container, url, client) = runtime.block_on(start_container());
        Self {
            container: Some(container),
            runtime,
            url,
            client,
        }
    }

    /// A running Postgres with risk's symbol tables and the eight seeded
    /// symbols.
    pub fn start_migrated() -> Self {
        let postgres = Self::start();
        postgres.migrate();
        postgres
    }

    pub fn migrate(&self) {
        for migration in SYMBOL_MIGRATIONS {
            self.runtime
                .block_on(self.client.batch_execute(migration))
                .expect("apply migration");
        }
    }

    pub fn execute(&self, sql: &str) {
        self.runtime
            .block_on(self.client.batch_execute(sql))
            .expect("execute sql");
    }
}

async fn start_container() -> (ContainerAsync<PostgresNoLogWait>, String, Client) {
    let container = PostgresNoLogWait::default()
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("mapped port for 5432");
    let url = format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    let deadline = Instant::now() + READY_TIMEOUT;
    let (client, connection) = loop {
        match tokio_postgres::connect(&url, NoTls).await {
            Ok(pair) => break pair,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "postgres on port {port} not ready within {READY_TIMEOUT:?}: {error}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    tokio::spawn(drive_connection(connection));
    (container, url, client)
}

async fn drive_connection(
    connection: tokio_postgres::Connection<
        tokio_postgres::Socket,
        tokio_postgres::tls::NoTlsStream,
    >,
) {
    if let Err(error) = connection.await {
        eprintln!("pg connection closed: {error}");
    }
}
