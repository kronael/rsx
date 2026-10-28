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
use rsx_risk::config::schedule_symbol_config;
use rsx_risk::config::ConfigField;
use rsx_risk::config::ScheduledVersion;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::persist::PersistEvent;
use rsx_risk::persist::StopMode;
use rsx_risk::schema::run_migrations;
use rsx_risk::SymbolRiskConfig;
use std::io::ErrorKind;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const FAR_FUTURE_MS: i64 = 4_102_444_800_000;
/// 1 USD in risk's notional unit: every preset has price_decimals +
/// qty_decimals = 10.
const ONE_USD: i64 = 10_000_000_000;

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
    assert_eq!(btc.config_version, 2);
    assert_eq!(btc.params.initial_margin_rate, 500);
    assert_eq!(btc.params.maintenance_margin_rate, 250);
    assert_eq!(btc.params.max_leverage, 20);
    assert_eq!(btc.taker_fee_bps, 5);
    assert_eq!(btc.maker_fee_bps, -1);
    assert_eq!(btc.params.min_notional, ONE_USD);

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
             SELECT symbol_id, 3, $1, tick_size, lot_size, price_decimals, \
                    qty_decimals, status, min_notional, max_order_qty, \
                    maker_fee_bps, 9, 4000, 2000, 2, funding_interval_sec, \
                    funding_rate_min_bps, funding_rate_max_bps, 0 \
             FROM symbol_config_schedule \
             WHERE symbol_id = 10 AND config_version = 2",
            &[&FAR_FUTURE_MS],
        )
        .await
        .expect("schedule v2");

    // Not effective yet: the active row stays v2.
    let now = load_symbol_configs(&client, 16, 1_000).await.expect("load");
    assert_eq!(now[3].config_version, 2);
    // Effective: v3 wins.
    let later = load_symbol_configs(&client, 16, FAR_FUTURE_MS as u64)
        .await
        .expect("load");
    assert_eq!(later[3].config_version, 3);
    assert_eq!(later[3].params.initial_margin_rate, 4000);
    assert_eq!(later[3].taker_fee_bps, 9);

    // The reload path fetches by version regardless of effective_at.
    let v3 = load_symbol_config(&client, 10, 3)
        .await
        .expect("load")
        .expect("row");
    assert_eq!(v3, later[3]);
    assert!(load_symbol_config(&client, 10, 4)
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
    let shutdown = Arc::new(AtomicU8::new(StopMode::Run as u8));
    let worker = tokio::spawn(run_persist_worker_with_shutdown(
        consumer,
        common::connect(port).await,
        0,
        Some(shutdown.clone()),
        Some(config_prod),
        None,
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

    shutdown.store(StopMode::FlushAndStop as u8, Ordering::Relaxed);
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

    insert_with_min_notional(&client, 10, 3, Some(2 * ONE_USD)).await;
    let v3 = load_symbol_config(&client, 10, 3)
        .await
        .expect("load")
        .expect("row");
    assert_eq!(v3.params.min_notional, 2 * ONE_USD);

    insert_with_min_notional(&client, 10, 4, None).await;
    let err = load_symbol_config(&client, 10, 4).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("min_notional is NULL"), "{err}");
    assert!(err.to_string().contains("symbol 10 v4"), "{err}");

    insert_with_min_notional(&client, 10, 5, Some(0)).await;
    let err = load_symbol_config(&client, 10, 5).await.unwrap_err();
    assert!(err.to_string().contains("min_notional is 0"), "{err}");

    // The boot load names the bad active row and fails whole.
    let err = load_symbol_configs(&client, 16, FAR_FUTURE_MS as u64)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("symbol 10 v5"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn config_reload_of_a_bad_row_ships_nothing() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    insert_with_min_notional(&client, 10, 3, None).await;
    insert_with_min_notional(&client, 10, 4, Some(5_000)).await;

    let (mut producer, consumer) = rtrb::RingBuffer::<PersistEvent>::new(16);
    let (config_prod, mut config_cons) = rtrb::RingBuffer::<SymbolRiskConfig>::new(4);
    let shutdown = Arc::new(AtomicU8::new(StopMode::Run as u8));
    let worker = tokio::spawn(run_persist_worker_with_shutdown(
        consumer,
        common::connect(port).await,
        0,
        Some(shutdown.clone()),
        Some(config_prod),
        None,
    ));
    for config_version in [3, 4] {
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
    // v3 (NULL minimum) was dropped; the next shipped config is v4.
    assert_eq!(config.config_version, 4);
    assert_eq!(config.params.min_notional, 5_000);
    assert!(config_cons.pop().is_err());

    shutdown.store(StopMode::FlushAndStop as u8, Ordering::Relaxed);
    worker.await.expect("worker exits");
}

const PRESET_IDS: [i32; 8] = [1, 2, 3, 10, 11, 12, 13, 14];

/// `(symbol_id, config_version, min_notional)` of every schedule row.
async fn schedule_minimums(client: &tokio_postgres::Client) -> Vec<(i32, i64, Option<i64>)> {
    client
        .query(
            "SELECT symbol_id, config_version, min_notional \
             FROM symbol_config_schedule ORDER BY symbol_id, config_version",
            &[],
        )
        .await
        .expect("schedule rows")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
}

/// Restores the state migrations 001-006 leave: version 1 only, no record
/// of 007, so the next `run_migrations` applies 007 again.
async fn rewind_min_order_notional(client: &tokio_postgres::Client) {
    client
        .batch_execute(
            "DELETE FROM symbol_config_schedule WHERE config_version > 1; \
             DELETE FROM migrations WHERE id = '007_min_order_notional'",
        )
        .await
        .expect("rewind 007");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn clean_start_gives_every_preset_a_one_usd_minimum() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");

    // Every preset has v1 (the seed's 1e-7 USD) and v2 (1 USD): 10^(price
    // decimals + qty decimals), taken from the row's own decimals.
    let rows = schedule_minimums(&client).await;
    assert_eq!(rows.len(), 16, "{rows:?}");
    for id in PRESET_IDS {
        let v1 = rows.iter().find(|r| r.0 == id && r.1 == 1).unwrap();
        let v2 = rows.iter().find(|r| r.0 == id && r.1 == 2).unwrap();
        assert_eq!(v1.2, Some(1000), "symbol {id} v1 is untouched");
        assert_eq!(v2.2, Some(ONE_USD), "symbol {id} v2");
    }

    // v2 is v1 with only version, minimum and created_at_ms changed.
    let differing: i64 = client
        .query_one(
            "SELECT count(*) FROM symbol_config_schedule a \
             JOIN symbol_config_schedule b USING (symbol_id) \
             WHERE a.config_version = 1 AND b.config_version = 2 \
               AND (a.effective_at_ms, a.tick_size, a.lot_size, \
                    a.price_decimals, a.qty_decimals, a.status, \
                    a.max_order_qty, a.maker_fee_bps, a.taker_fee_bps, \
                    a.initial_margin_rate_bps, a.maintenance_margin_rate_bps, \
                    a.max_leverage, a.funding_interval_sec, \
                    a.funding_rate_min_bps, a.funding_rate_max_bps) \
                   IS DISTINCT FROM \
                   (b.effective_at_ms, b.tick_size, b.lot_size, \
                    b.price_decimals, b.qty_decimals, b.status, \
                    b.max_order_qty, b.maker_fee_bps, b.taker_fee_bps, \
                    b.initial_margin_rate_bps, b.maintenance_margin_rate_bps, \
                    b.max_leverage, b.funding_interval_sec, \
                    b.funding_rate_min_bps, b.funding_rate_max_bps)",
            &[],
        )
        .await
        .expect("compare")
        .get(0);
    assert_eq!(differing, 0);

    // The boot load sees 1 USD on every preset.
    let configs = load_symbol_configs(&client, 16, 1_000).await.expect("load");
    assert_eq!(configs.len(), PRESET_IDS.len());
    for config in &configs {
        assert_eq!(config.config_version, 2, "symbol {}", config.symbol_id);
        assert_eq!(config.params.min_notional, ONE_USD);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn running_the_migrations_twice_changes_nothing() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    let first = schedule_minimums(&client).await;

    // The version guard skips 007.
    run_migrations(&client).await.expect("second run");
    assert_eq!(schedule_minimums(&client).await, first);

    // Without the guard record the rows themselves make 007 a no-op: every
    // symbol's highest version is already at the minimum.
    client
        .execute(
            "DELETE FROM migrations WHERE id = '007_min_order_notional'",
            &[],
        )
        .await
        .expect("drop record");
    run_migrations(&client).await.expect("third run");
    assert_eq!(schedule_minimums(&client).await, first);
    let recorded: i64 = client
        .query_one(
            "SELECT count(*) FROM migrations WHERE id = '007_min_order_notional'",
            &[],
        )
        .await
        .expect("count")
        .get(0);
    assert_eq!(recorded, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn operator_minimums_are_only_raised_never_lowered() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    rewind_min_order_notional(&client).await;

    // BTC: an operator already set 5 USD. ETH: 0.5 USD, below the floor.
    // SOL: a future-dated version below the floor. PENGU: a NULL minimum.
    // WIF: 1 USD exactly.
    insert_with_min_notional(&client, 1, 2, Some(5 * ONE_USD)).await;
    insert_with_min_notional(&client, 2, 2, Some(ONE_USD / 2)).await;
    insert_with_min_notional(&client, 3, 2, Some(1000)).await;
    client
        .execute(
            "UPDATE symbol_config_schedule SET effective_at_ms = $1 \
             WHERE symbol_id = 3 AND config_version = 2",
            &[&FAR_FUTURE_MS],
        )
        .await
        .expect("future-date SOL v2");
    insert_with_min_notional(&client, 10, 2, None).await;
    insert_with_min_notional(&client, 11, 2, Some(ONE_USD)).await;

    run_migrations(&client).await.expect("migrations");

    let rows = schedule_minimums(&client).await;
    let versions = |id: i32| -> Vec<(i64, Option<i64>)> {
        rows.iter()
            .filter(|r| r.0 == id)
            .map(|r| (r.1, r.2))
            .collect()
    };
    // Kept: no new version.
    assert_eq!(versions(1), vec![(1, Some(1000)), (2, Some(5 * ONE_USD))]);
    assert_eq!(versions(11), vec![(1, Some(1000)), (2, Some(ONE_USD))]);
    // Raised: a new version on top of the operator's, which stays.
    assert_eq!(
        versions(2),
        vec![(1, Some(1000)), (2, Some(ONE_USD / 2)), (3, Some(ONE_USD))]
    );
    assert_eq!(
        versions(10),
        vec![(1, Some(1000)), (2, None), (3, Some(ONE_USD))]
    );
    // The base is the highest version, so the future-dated SOL v2 is not
    // overtaken: v3 copies its effective time.
    assert_eq!(
        versions(3),
        vec![(1, Some(1000)), (2, Some(1000)), (3, Some(ONE_USD))]
    );
    let sol_v3_at: i64 = client
        .query_one(
            "SELECT effective_at_ms FROM symbol_config_schedule \
             WHERE symbol_id = 3 AND config_version = 3",
            &[],
        )
        .await
        .expect("sol v3")
        .get(0);
    assert_eq!(sol_v3_at, FAR_FUTURE_MS);
    // Untouched presets get the plain v2.
    assert_eq!(versions(12), vec![(1, Some(1000)), (2, Some(ONE_USD))]);

    let active = load_symbol_configs(&client, 16, 1_000).await.expect("load");
    let minimum = |id: u32| {
        active
            .iter()
            .find(|c| c.symbol_id == id)
            .unwrap()
            .params
            .min_notional
    };
    assert_eq!(minimum(1), 5 * ONE_USD);
    assert_eq!(minimum(2), ONE_USD);
    assert_eq!(minimum(10), ONE_USD);
    assert_eq!(minimum(11), ONE_USD);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn a_row_off_the_shared_notional_unit_fails_the_load() {
    let (_container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");

    // PENGU v3 with 3 + 6 decimals: its price * qty would be in 1e-9 quote.
    client
        .execute(
            "INSERT INTO symbol_config_schedule \
             SELECT symbol_id, 3, effective_at_ms, tick_size, lot_size, \
                    3, 6, status, min_notional, max_order_qty, \
                    maker_fee_bps, taker_fee_bps, initial_margin_rate_bps, \
                    maintenance_margin_rate_bps, max_leverage, \
                    funding_interval_sec, funding_rate_min_bps, \
                    funding_rate_max_bps, 0 \
             FROM symbol_config_schedule \
             WHERE symbol_id = 10 AND config_version = 2",
            &[],
        )
        .await
        .expect("insert v3");

    let err = load_symbol_config(&client, 10, 3).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let text = err.to_string();
    assert!(text.contains("symbol 10 v3"), "{text}");
    assert!(
        text.contains("price_decimals 3 + qty_decimals 6 = 9"),
        "{text}"
    );
    assert!(text.contains("needs 10"), "{text}");

    // The boot load fails whole rather than skipping the symbol.
    let err = load_symbol_configs(&client, 16, 1_000).await.unwrap_err();
    assert!(err.to_string().contains("symbol 10 v3"), "{err}");

    // v2 is still loadable by version: only the bad row is refused.
    let v2 = load_symbol_config(&client, 10, 2).await.expect("v2");
    assert!(v2.is_some());
}

async fn schedule_rows(client: &tokio_postgres::Client, symbol_id: i32) -> Vec<String> {
    client
        .query(
            "SELECT row_to_json(c)::text FROM symbol_config_schedule c \
             WHERE symbol_id = $1 ORDER BY config_version",
            &[&symbol_id],
        )
        .await
        .expect("read schedule")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn schedule_inserts_the_next_version_and_leaves_every_earlier_row_alone() {
    let (_container, port) = common::start_postgres().await;
    let mut client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    let before = schedule_rows(&client, 10).await;
    assert_eq!(before.len(), 2);

    let scheduled = schedule_symbol_config(
        &mut client,
        10,
        &[
            (ConfigField::InitialMarginRateBps, 2000),
            (ConfigField::MaintenanceMarginRateBps, 1000),
            (ConfigField::MaxLeverage, 5),
            (ConfigField::MakerFeeBps, -2),
        ],
        FAR_FUTURE_MS as u64,
    )
    .await
    .expect("schedule");
    assert_eq!(
        scheduled,
        ScheduledVersion {
            symbol_id: 10,
            config_version: 3,
            effective_at_ms: FAR_FUTURE_MS as u64,
        }
    );

    let after = schedule_rows(&client, 10).await;
    assert_eq!(after.len(), 3);
    assert_eq!(after[..2], before[..], "earlier rows are untouched");

    let row = client
        .query_one(
            "SELECT effective_at_ms, tick_size, lot_size, price_decimals, \
                    qty_decimals, status, min_notional, taker_fee_bps, \
                    maker_fee_bps, initial_margin_rate_bps, \
                    maintenance_margin_rate_bps, max_leverage, \
                    funding_interval_sec, created_at_ms \
             FROM symbol_config_schedule WHERE symbol_id = 10 AND config_version = 3",
            &[],
        )
        .await
        .expect("new row");
    assert_eq!(row.get::<_, i64>(0), FAR_FUTURE_MS);
    assert_eq!(row.get::<_, i64>(1), 1, "tick copied");
    assert_eq!(row.get::<_, i64>(2), 100_000, "lot copied");
    assert_eq!(row.get::<_, i16>(3), 6);
    assert_eq!(row.get::<_, i16>(4), 4);
    assert_eq!(row.get::<_, String>(5), "trading");
    assert_eq!(row.get::<_, i64>(6), ONE_USD, "min_notional copied");
    assert_eq!(row.get::<_, i32>(7), 5, "taker fee copied");
    assert_eq!(row.get::<_, i32>(8), -2);
    assert_eq!(row.get::<_, i32>(9), 2000);
    assert_eq!(row.get::<_, i32>(10), 1000);
    assert_eq!(row.get::<_, i32>(11), 5);
    assert_eq!(row.get::<_, i32>(12), 28800, "funding interval copied");
    assert!(row.get::<_, i64>(13) > 0, "created_at_ms is stamped");

    let now = load_symbol_configs(&client, 16, 1_000).await.expect("load");
    assert_eq!(now[3].config_version, 2);
    let later = load_symbol_configs(&client, 16, FAR_FUTURE_MS as u64)
        .await
        .expect("load");
    assert_eq!(later[3].config_version, 3);
    assert_eq!(later[3].params.max_leverage, 5);
    assert_eq!(later[3].maker_fee_bps, -2);

    let second = schedule_symbol_config(
        &mut client,
        10,
        &[(ConfigField::MinNotional, 3 * ONE_USD)],
        FAR_FUTURE_MS as u64,
    )
    .await
    .expect("schedule again");
    assert_eq!(second.config_version, 4);
    let v4 = load_symbol_config(&client, 10, 4)
        .await
        .expect("load")
        .expect("row");
    assert_eq!(v4.params.min_notional, 3 * ONE_USD);
    assert_eq!(
        v4.params.initial_margin_rate, 2000,
        "copies the highest version"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn schedule_dated_before_a_pending_version_is_refused() {
    let (_container, port) = common::start_postgres().await;
    let mut client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    let in_an_hour = FAR_FUTURE_MS as u64;
    schedule_symbol_config(
        &mut client,
        10,
        &[(ConfigField::MaxLeverage, 5)],
        in_an_hour,
    )
    .await
    .expect("schedule v3");
    let before = schedule_rows(&client, 10).await;

    let error = schedule_symbol_config(&mut client, 10, &[(ConfigField::TakerFeeBps, 7)], 1_000)
        .await
        .expect_err("earlier than v3");
    assert_eq!(error.kind(), ErrorKind::InvalidInput, "{error}");
    assert!(error.to_string().contains("v3"), "{error}");
    assert_eq!(schedule_rows(&client, 10).await, before);

    let at_pending = schedule_symbol_config(
        &mut client,
        10,
        &[(ConfigField::TakerFeeBps, 7)],
        in_an_hour,
    )
    .await
    .expect("at v3");
    assert_eq!(at_pending.config_version, 4);
    let v4 = load_symbol_config(&client, 10, 4)
        .await
        .expect("load")
        .expect("row");
    assert_eq!(v4.params.max_leverage, 5, "v4 copies v3");
    assert_eq!(v4.taker_fee_bps, 7);
}

type Changes<'a> = &'a [(ConfigField, i64)];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn schedule_that_risk_would_refuse_writes_nothing() {
    let (_container, port) = common::start_postgres().await;
    let mut client = common::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    let before = schedule_rows(&client, 10).await;

    let refused: [(Changes, &str); 5] = [
        (
            &[(ConfigField::MaintenanceMarginRateBps, 1000)],
            "must be below initial_margin_rate_bps",
        ),
        (
            &[
                (ConfigField::InitialMarginRateBps, 100),
                (ConfigField::MaintenanceMarginRateBps, 200),
            ],
            "must be below initial_margin_rate_bps",
        ),
        (&[(ConfigField::MaxLeverage, 0)], "max_leverage is 0"),
        (&[(ConfigField::MinNotional, 0)], "min_notional is 0"),
        (
            &[(ConfigField::TakerFeeBps, 1), (ConfigField::TakerFeeBps, 2)],
            "taker_fee_bps is set twice",
        ),
    ];
    for (changes, message) in refused {
        let error = schedule_symbol_config(&mut client, 10, changes, 0)
            .await
            .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{error}");
        assert!(error.to_string().contains(message), "{error}");
    }

    let unknown = schedule_symbol_config(&mut client, 99, &[(ConfigField::MaxLeverage, 3)], 0)
        .await
        .expect_err("unknown symbol");
    assert_eq!(unknown.kind(), ErrorKind::NotFound, "{unknown}");

    assert_eq!(schedule_rows(&client, 10).await, before);
    assert!(schedule_rows(&client, 99).await.is_empty());
}
