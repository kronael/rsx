//! Symbol risk config comes from `symbol_config_schedule`: the seed
//! migration must leave every playground symbol tradeable, a
//! scheduled future version must stay invisible until it is
//! effective, and a `CONFIG_APPLIED` version must be fetchable by
//! `(symbol_id, config_version)`.
//!
//! Needs Docker; `#[ignore]`, run under `make integration`.

mod common;

use rsx_risk::config::load_symbol_config;
use rsx_risk::config::load_symbol_configs;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::persist::PersistEvent;
use rsx_risk::schema::run_migrations;
use rsx_risk::SymbolRiskConfig;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const FAR_FUTURE_MS: i64 = 4_102_444_800_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn seeded_symbols_load_with_future_versions_excluded() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");

    let configs = load_symbol_configs(&client, 16, 1_000).await.expect("load");
    let ids: Vec<u32> = configs.iter().map(|c| c.symbol_id).collect();
    assert_eq!(ids, vec![1, 2, 3, 10, 11, 12, 13, 14]);

    let btc = &configs[0];
    assert_eq!(btc.config_version, 1);
    assert_eq!(btc.params.initial_margin_rate, 500);
    assert_eq!(btc.params.maintenance_margin_rate, 250);
    assert_eq!(btc.params.max_leverage, 20);
    assert_eq!(btc.taker_fee_bps, 5);
    assert_eq!(btc.maker_fee_bps, -1);
    assert_eq!(btc.params.min_notional, 1000);

    let pengu = &configs[3];
    assert_eq!(pengu.symbol_id, 10);
    assert_eq!(pengu.params.initial_margin_rate, 1000);
    assert_eq!(pengu.params.max_leverage, 10);

    // max_symbols bounds the load: ids >= max_symbols are absent.
    let small = load_symbol_configs(&client, 4, 1_000).await.expect("load");
    assert_eq!(small.len(), 3);

    client
        .execute(
            "INSERT INTO symbol_config_schedule \
             SELECT symbol_id, 2, $1, tick_size, lot_size, price_decimals, \
                    qty_decimals, status, min_notional, max_order_qty, \
                    maker_fee_bps, 9, 4000, 2000, 2, funding_interval_sec, \
                    funding_rate_min_bps, funding_rate_max_bps, 0 \
             FROM symbol_config_schedule WHERE symbol_id = 10",
            &[&FAR_FUTURE_MS],
        )
        .await
        .expect("schedule v2");

    // Not effective yet: the active row stays v1.
    let now = load_symbol_configs(&client, 16, 1_000).await.expect("load");
    assert_eq!(now[3].config_version, 1);
    // Effective: v2 wins.
    let later = load_symbol_configs(&client, 16, FAR_FUTURE_MS as u64)
        .await
        .expect("load");
    assert_eq!(later[3].config_version, 2);
    assert_eq!(later[3].params.initial_margin_rate, 4000);
    assert_eq!(later[3].taker_fee_bps, 9);

    // The reload path fetches by version regardless of effective_at.
    let v2 = load_symbol_config(&client, 10, 2)
        .await
        .expect("load")
        .expect("row");
    assert_eq!(v2, later[3]);
    assert!(load_symbol_config(&client, 10, 3)
        .await
        .expect("load")
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn persist_worker_answers_config_reload_over_config_ring() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");

    let (mut producer, consumer) = rtrb::RingBuffer::<PersistEvent>::new(16);
    let (config_prod, mut config_cons) = rtrb::RingBuffer::<SymbolRiskConfig>::new(4);
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker = tokio::spawn(run_persist_worker_with_shutdown(
        consumer,
        common::connect(port).await,
        0,
        Some(shutdown.clone()),
        Some(config_prod),
    ));

    producer
        .push(PersistEvent::ConfigReload {
            symbol_id: 10,
            config_version: 1,
        })
        .unwrap();
    producer
        .push(PersistEvent::ConfigReload {
            symbol_id: 10,
            config_version: 99,
        })
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let config = loop {
        if let Ok(config) = config_cons.pop() {
            break config;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no config within 10s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(config.symbol_id, 10);
    assert_eq!(config.config_version, 1);
    assert_eq!(config.params.max_leverage, 10);

    // The unknown version is logged and dropped, never shipped.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(config_cons.pop().is_err());

    shutdown.store(true, Ordering::Relaxed);
    worker.await.expect("worker exits");
}

/// Inserts version `version` of `symbol_id` copying v1 with the given
/// `min_notional` (`NULL` when `None`).
async fn insert_with_min_notional(
    client: &tokio_postgres::Client,
    symbol_id: i32,
    version: i64,
    min_notional: Option<i64>,
) {
    client
        .execute(
            "INSERT INTO symbol_config_schedule \
             SELECT symbol_id, $2, effective_at_ms, tick_size, lot_size, \
                    price_decimals, qty_decimals, status, $3, max_order_qty, \
                    maker_fee_bps, taker_fee_bps, initial_margin_rate_bps, \
                    maintenance_margin_rate_bps, max_leverage, \
                    funding_interval_sec, funding_rate_min_bps, \
                    funding_rate_max_bps, 0 \
             FROM symbol_config_schedule \
             WHERE symbol_id = $1 AND config_version = 1",
            &[&symbol_id, &version, &min_notional],
        )
        .await
        .expect("insert version");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn min_notional_is_loaded_and_a_bad_row_fails_the_load() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");

    insert_with_min_notional(&client, 10, 2, Some(10_000_000_000)).await;
    let v2 = load_symbol_config(&client, 10, 2)
        .await
        .expect("load")
        .expect("row");
    assert_eq!(v2.params.min_notional, 10_000_000_000);

    insert_with_min_notional(&client, 10, 3, None).await;
    let err = load_symbol_config(&client, 10, 3).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("min_notional is NULL"), "{err}");
    assert!(err.to_string().contains("symbol 10 v3"), "{err}");

    insert_with_min_notional(&client, 10, 4, Some(0)).await;
    let err = load_symbol_config(&client, 10, 4).await.unwrap_err();
    assert!(err.to_string().contains("min_notional is 0"), "{err}");

    // The boot load names the bad active row and fails whole.
    let err = load_symbol_configs(&client, 16, FAR_FUTURE_MS as u64)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("symbol 10 v4"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn config_reload_of_a_bad_row_ships_nothing() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    insert_with_min_notional(&client, 10, 2, None).await;
    insert_with_min_notional(&client, 10, 3, Some(5_000)).await;

    let (mut producer, consumer) = rtrb::RingBuffer::<PersistEvent>::new(16);
    let (config_prod, mut config_cons) = rtrb::RingBuffer::<SymbolRiskConfig>::new(4);
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker = tokio::spawn(run_persist_worker_with_shutdown(
        consumer,
        common::connect(port).await,
        0,
        Some(shutdown.clone()),
        Some(config_prod),
    ));
    for config_version in [2, 3] {
        producer
            .push(PersistEvent::ConfigReload {
                symbol_id: 10,
                config_version,
            })
            .unwrap();
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let config = loop {
        if let Ok(config) = config_cons.pop() {
            break config;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no config within 10s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // v2 (NULL minimum) was dropped; the next shipped config is v3.
    assert_eq!(config.config_version, 3);
    assert_eq!(config.params.min_notional, 5_000);
    assert!(config_cons.pop().is_err());

    shutdown.store(true, Ordering::Relaxed);
    worker.await.expect("worker exits");
}
