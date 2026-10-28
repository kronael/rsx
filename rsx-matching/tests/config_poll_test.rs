mod common;

use crate::common::PostgresNoLogWait;
use rsx_matching::config::boot_config;
use rsx_matching::config::config_link;
use rsx_matching::config::load_applied_config;
use rsx_matching::config::poll_scheduled_configs;
use rsx_matching::config::run_config_sidecar;
use rsx_matching::config::write_applied_config;
use rsx_matching::config::ScheduledConfig;
use rsx_matching::recovery::recover;
use rsx_types::SymbolConfig;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

async fn setup_db() -> (
    testcontainers::ContainerAsync<PostgresNoLogWait>,
    tokio_postgres::Client,
) {
    let (container, port) = common::start_postgres().await;
    let client = common::connect(port).await;

    let migration = include_str!("../../rsx-risk/migrations/005_symbol_config.sql");
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

    client
        .execute(
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
        )
        .await
        .expect("insert");

    let configs = poll_scheduled_configs(&client, symbol_id, 0, now)
        .await
        .expect("poll");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].config_version, 1);
    assert_eq!(configs[0].config.tick_size, 1);
    assert_eq!(configs[0].config.lot_size, 1000);
}

#[tokio::test]
#[ignore]
async fn poll_ignores_future_configs() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let future = now + 60_000;

    client
        .execute(
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
        )
        .await
        .expect("insert");

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
        client
            .execute(
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
            )
            .await
            .expect("insert");
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
        client
            .execute(
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
            )
            .await
            .expect("insert");
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
    let ts_ns = now * 1_000_000;

    let cfg = ScheduledConfig {
        config: SymbolConfig {
            symbol_id,
            tick_size: 1,
            lot_size: 1000,
            price_decimals: 8,
            qty_decimals: 8,
        },
        config_version: 1,
        effective_at_ms: now,
    };

    write_applied_config(&client, symbol_id, &cfg, ts_ns)
        .await
        .expect("write");

    let loaded = load_applied_config(&client, symbol_id)
        .await
        .expect("load")
        .expect("config");

    assert_eq!(loaded.config_version, 1);
    assert_eq!(loaded.config.tick_size, 1);
    assert_eq!(loaded.config.lot_size, 1000);
}

#[tokio::test]
#[ignore]
async fn write_applied_config_updates_existing() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;
    let now = now_ms();
    let ts_ns = now * 1_000_000;

    let cfg1 = ScheduledConfig {
        config: SymbolConfig {
            symbol_id,
            tick_size: 1,
            lot_size: 1000,
            price_decimals: 8,
            qty_decimals: 8,
        },
        config_version: 1,
        effective_at_ms: now,
    };
    write_applied_config(&client, symbol_id, &cfg1, ts_ns)
        .await
        .expect("write1");

    let cfg2 = ScheduledConfig {
        config: SymbolConfig {
            symbol_id,
            tick_size: 10,
            lot_size: 10000,
            price_decimals: 6,
            qty_decimals: 6,
        },
        config_version: 2,
        effective_at_ms: now + 1000,
    };
    write_applied_config(&client, symbol_id, &cfg2, ts_ns + 1000)
        .await
        .expect("write2");

    let loaded = load_applied_config(&client, symbol_id)
        .await
        .expect("load")
        .expect("config");

    assert_eq!(loaded.config_version, 2);
    assert_eq!(loaded.config.tick_size, 10);
    assert_eq!(loaded.config.lot_size, 10000);
}

#[tokio::test]
#[ignore]
async fn load_applied_config_returns_none_when_empty() {
    let (_c, client) = setup_db().await;
    let symbol_id = 1u32;

    let loaded = load_applied_config(&client, symbol_id).await.expect("load");

    assert!(loaded.is_none());
}

fn env_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        tick_size: 1,
        lot_size: 1000,
        price_decimals: 8,
        qty_decimals: 8,
    }
}

async fn write_applied_v2(client: &tokio_postgres::Client) {
    let applied = ScheduledConfig {
        config: SymbolConfig {
            symbol_id: 1,
            tick_size: 10,
            lot_size: 5000,
            price_decimals: 6,
            qty_decimals: 4,
        },
        config_version: 2,
        effective_at_ms: now_ms(),
    };
    write_applied_config(client, 1, &applied, now_ms() * 1_000_000)
        .await
        .expect("write");
}

#[tokio::test]
#[ignore]
async fn boot_config_prefers_the_applied_row_over_env() {
    let (_c, client) = setup_db().await;
    write_applied_v2(&client).await;

    let boot = boot_config(&client, env_config()).await.expect("boot");

    assert_eq!(boot.applied_version, Some(2));
    assert_eq!(boot.config.symbol_id, 1);
    assert_eq!(boot.config.tick_size, 10);
    assert_eq!(boot.config.lot_size, 5000);
    assert_eq!(boot.config.price_decimals, 6);
    assert_eq!(boot.config.qty_decimals, 4);
}

#[tokio::test]
#[ignore]
async fn boot_config_uses_env_when_no_row_is_applied() {
    let (_c, client) = setup_db().await;

    let boot = boot_config(&client, env_config()).await.expect("boot");

    assert_eq!(boot.applied_version, None);
    assert_eq!(boot.config.tick_size, 1);
    assert_eq!(boot.config.lot_size, 1000);
}

#[tokio::test]
#[ignore]
async fn boot_config_uses_env_when_risk_has_not_migrated_yet() {
    let (_c, port) = common::start_postgres().await;
    let client = common::connect(port).await;

    let boot = boot_config(&client, env_config()).await.expect("boot");

    assert_eq!(boot.applied_version, None);
    assert_eq!(boot.config.tick_size, 1);
}

#[tokio::test]
#[ignore]
async fn boot_config_fails_when_the_applied_row_cannot_be_read() {
    let (_c, client) = setup_db().await;
    client
        .batch_execute("ALTER TABLE symbol_config_applied DROP COLUMN lot_size")
        .await
        .unwrap();

    let error = boot_config(&client, env_config())
        .await
        .expect_err("a broken applied table must stop boot");

    assert!(error.to_string().contains("load_applied_config"), "{error}");
}

#[tokio::test]
#[ignore]
async fn restart_without_a_checkpoint_runs_the_applied_tick_despite_env_v1() {
    let (_c, client) = setup_db().await;
    write_applied_v2(&client).await;
    let dir = TempDir::new().unwrap();
    let wal_dir = dir.path().to_str().unwrap();

    let boot = boot_config(&client, env_config()).await.expect("boot");
    let recovered = recover(boot, wal_dir, 1).expect("recover");

    assert_eq!(recovered.book.config.tick_size, 10);
    assert_eq!(recovered.book.config.lot_size, 5000);
    assert_eq!(recovered.next_seq, 1);
}

async fn insert_due_schedule(client: &tokio_postgres::Client, config_version: i64) {
    let now = now_ms();
    client
        .execute(
            "INSERT INTO symbol_config_schedule \
         (symbol_id, config_version, effective_at_ms, tick_size, lot_size, \
          price_decimals, qty_decimals, status, created_at_ms) \
         VALUES (1, $1, $2, $3, 1000, 8, 8, 'active', $2)",
            &[
                &config_version,
                &((now - 60_000) as i64),
                &(config_version * 10),
            ],
        )
        .await
        .expect("insert");
}

#[tokio::test]
#[ignore]
async fn sidecar_hands_over_each_due_config_once_and_writes_the_applied_row() {
    let (container, client) = setup_db().await;
    insert_due_schedule(&client, 1).await;
    let (mut loop_end, sidecar_end) = config_link();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let sidecar_client = common::connect(port).await;
    let sidecar = tokio::spawn(run_config_sidecar(
        sidecar_client,
        1,
        0,
        Duration::from_millis(50),
        sidecar_end,
    ));

    let deadline = Instant::now() + Duration::from_secs(10);
    let due = loop {
        if let Ok(due) = loop_end.due.pop() {
            break due;
        }
        assert!(Instant::now() < deadline, "no config handed over");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(due.config_version, 1);
    assert_eq!(due.config.tick_size, 10);
    loop_end.applied.push((due, now_ms() * 1_000_000)).unwrap();

    let applied = loop {
        if let Some(applied) = load_applied_config(&client, 1).await.expect("load") {
            break applied;
        }
        assert!(Instant::now() < deadline, "no applied row written");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(applied.config_version, 1);
    assert_eq!(applied.config.tick_size, 10);

    // The cursor moved past v1: several more polls hand over nothing.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(loop_end.due.pop().is_err());
    insert_due_schedule(&client, 2).await;
    let second = loop {
        if let Ok(due) = loop_end.due.pop() {
            break due;
        }
        assert!(Instant::now() < deadline, "v2 not handed over");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(second.config_version, 2);

    drop(loop_end);
    tokio::time::timeout(Duration::from_secs(5), sidecar)
        .await
        .expect("the sidecar stops when the loop drops its end")
        .unwrap();
}

#[test]
fn scheduled_config_composes_symbol_config() {
    let cfg = ScheduledConfig {
        config: SymbolConfig {
            symbol_id: 42,
            tick_size: 10,
            lot_size: 1000,
            price_decimals: 8,
            qty_decimals: 6,
        },
        config_version: 1,
        effective_at_ms: 0,
    };

    assert_eq!(cfg.config.symbol_id, 42);
    assert_eq!(cfg.config.tick_size, 10);
    assert_eq!(cfg.config.lot_size, 1000);
    assert_eq!(cfg.config.price_decimals, 8);
    assert_eq!(cfg.config.qty_decimals, 6);
    assert_eq!(cfg.config_version, 1);
}
