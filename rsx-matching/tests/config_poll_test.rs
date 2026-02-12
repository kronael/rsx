use rsx_matching::config::load_applied_config;
use rsx_matching::config::poll_scheduled_configs;
use rsx_matching::config::write_applied_config;
use rsx_matching::config::ScheduledConfig;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::NoTls;

async fn setup_db() -> (
    testcontainers::ContainerAsync<Postgres>,
    tokio_postgres::Client,
) {
    let container: testcontainers::ContainerAsync<Postgres> =
        Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connstr = format!(
        "host=localhost port={port} user=postgres \
         password=postgres dbname=postgres"
    );
    let (client, conn) =
        tokio_postgres::connect(&connstr, NoTls)
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let migration = include_str!("../migrations/001_symbol_config.sql");
    client.batch_execute(migration).await.unwrap();

    client
        .execute("DELETE FROM symbol_config_schedule", &[])
        .await
        .unwrap();
    client
        .execute("DELETE FROM symbol_config_applied", &[])
        .await
        .unwrap();

    (container, client)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test]
#[ignore]
async fn poll_returns_empty_when_no_configs() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let current_version = 0u64;
    let now = now_ms();

    let configs = poll_scheduled_configs(&client, symbol_id, current_version, now)
        .await
        .expect("poll");

    assert_eq!(configs.len(), 0);
}

#[tokio::test]
#[ignore]
async fn poll_returns_config_when_effective() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let past = now - 60_000;

    client.execute(
        "INSERT INTO symbol_config_schedule \
         (symbol_id, config_version, effective_at_ms, tick_size, lot_size, \
          price_decimals, qty_decimals, status, created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &(symbol_id as i32),
            &1i64,
            &(past as i64),
            &1i64,
            &1000i64,
            &8i16,
            &8i16,
            &"active",
            &(now as i64),
        ],
    ).await.expect("insert");

    let configs = poll_scheduled_configs(&client, symbol_id, 0, now)
        .await
        .expect("poll");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].config_version, 1);
    assert_eq!(configs[0].tick_size, 1);
    assert_eq!(configs[0].lot_size, 1000);
}

#[tokio::test]
#[ignore]
async fn poll_ignores_future_configs() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let future = now + 60_000;

    client.execute(
        "INSERT INTO symbol_config_schedule \
         (symbol_id, config_version, effective_at_ms, tick_size, lot_size, \
          price_decimals, qty_decimals, status, created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &(symbol_id as i32),
            &1i64,
            &(future as i64),
            &1i64,
            &1000i64,
            &8i16,
            &8i16,
            &"active",
            &(now as i64),
        ],
    ).await.expect("insert");

    let configs = poll_scheduled_configs(&client, symbol_id, 0, now)
        .await
        .expect("poll");

    assert_eq!(configs.len(), 0);
}

#[tokio::test]
#[ignore]
async fn poll_returns_multiple_configs_in_order() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let past1 = now - 120_000;
    let past2 = now - 60_000;

    for (version, effective) in [(1, past1), (2, past2)] {
        client.execute(
            "INSERT INTO symbol_config_schedule \
             (symbol_id, config_version, effective_at_ms, tick_size, lot_size, \
              price_decimals, qty_decimals, status, created_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &(symbol_id as i32),
                &(version as i64),
                &(effective as i64),
                &(version as i64),
                &1000i64,
                &8i16,
                &8i16,
                &"active",
                &(now as i64),
            ],
        ).await.expect("insert");
    }

    let configs = poll_scheduled_configs(&client, symbol_id, 0, now)
        .await
        .expect("poll");

    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].config_version, 1);
    assert_eq!(configs[1].config_version, 2);
}

#[tokio::test]
#[ignore]
async fn poll_filters_by_current_version() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let past = now - 60_000;

    for version in 1..=3 {
        client.execute(
            "INSERT INTO symbol_config_schedule \
             (symbol_id, config_version, effective_at_ms, tick_size, lot_size, \
              price_decimals, qty_decimals, status, created_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &(symbol_id as i32),
                &(version as i64),
                &(past as i64),
                &1i64,
                &1000i64,
                &8i16,
                &8i16,
                &"active",
                &(now as i64),
            ],
        ).await.expect("insert");
    }

    let configs = poll_scheduled_configs(&client, symbol_id, 2, now)
        .await
        .expect("poll");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].config_version, 3);
}

#[tokio::test]
#[ignore]
async fn write_applied_config_inserts_new() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let ts_ns = (now * 1_000_000) as u64;

    let cfg = ScheduledConfig {
        config_version: 1,
        effective_at_ms: now,
        tick_size: 1,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 8,
    };

    write_applied_config(&client, symbol_id, &cfg, ts_ns)
        .await
        .expect("write");

    let loaded = load_applied_config(&client, symbol_id)
        .await
        .expect("load")
        .expect("config");

    assert_eq!(loaded.config_version, 1);
    assert_eq!(loaded.tick_size, 1);
    assert_eq!(loaded.lot_size, 1000);
}

#[tokio::test]
#[ignore]
async fn write_applied_config_updates_existing() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let ts_ns = (now * 1_000_000) as u64;

    let cfg1 = ScheduledConfig {
        config_version: 1,
        effective_at_ms: now,
        tick_size: 1,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 8,
    };
    write_applied_config(&client, symbol_id, &cfg1, ts_ns)
        .await
        .expect("write1");

    let cfg2 = ScheduledConfig {
        config_version: 2,
        effective_at_ms: now + 1000,
        tick_size: 10,
        lot_size: 10000,
        price_decimals: 6,
        qty_decimals: 6,
    };
    write_applied_config(&client, symbol_id, &cfg2, ts_ns + 1000)
        .await
        .expect("write2");

    let loaded = load_applied_config(&client, symbol_id)
        .await
        .expect("load")
        .expect("config");

    assert_eq!(loaded.config_version, 2);
    assert_eq!(loaded.tick_size, 10);
    assert_eq!(loaded.lot_size, 10000);
}

#[tokio::test]
#[ignore]
async fn load_applied_config_returns_none_when_empty() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;

    let loaded = load_applied_config(&client, symbol_id)
        .await
        .expect("load");

    assert!(loaded.is_none());
}

#[tokio::test]
async fn scheduled_config_to_symbol_config() {
    let cfg = ScheduledConfig {
        config_version: 1,
        effective_at_ms: 0,
        tick_size: 10,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 6,
    };

    let symbol_cfg = cfg.to_symbol_config(42);

    assert_eq!(symbol_cfg.symbol_id, 42);
    assert_eq!(symbol_cfg.tick_size, 10);
    assert_eq!(symbol_cfg.lot_size, 1000);
    assert_eq!(symbol_cfg.price_decimals, 8);
    assert_eq!(symbol_cfg.qty_decimals, 6);
}
