//! Funding settles each interval exactly once per symbol across
//! restarts and failover. Postgres paths: run under `make integration`.

use rsx_risk::persist::flush_batch;
use rsx_risk::persist::PersistEvent;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::schema::run_migrations;
use rsx_risk::Account;
use rsx_risk::FillEvent;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rtrb::Consumer;
use tokio_postgres::Client;

mod common;

const INTERVAL_SECS: u64 = 28_800;
const COLLATERAL: i64 = 1_000_000_000;
const PER_INTERVAL: i64 = 10_200;

async fn migrated_pg() -> (
    testcontainers::ContainerAsync<common::PostgresNoLogWait>,
    Client,
) {
    let (container, port) = common::start_postgres().await;
    let client = common::connect(port).await;
    run_migrations(&client).await.unwrap();
    (container, client)
}

fn shard_config() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: 4,
        symbol_params: vec![
            Some(SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
                min_notional: 1,
            });
            4
        ],
        taker_fee_bps: vec![0; 4],
        maker_fee_bps: vec![0; 4],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    }
}

fn set_prices(shard: &mut RiskShard) {
    shard.update_mark(0, 10_200);
    shard.index_prices[0].price = 10_000;
    shard.index_prices[0].valid = true;
}

/// A shard with a persist ring and user 0 long / user 1 short 100.
fn main_with_positions() -> (RiskShard, Consumer<PersistEvent>) {
    let mut shard = RiskShard::new(shard_config());
    let (producer, consumer) = rtrb::RingBuffer::<PersistEvent>::new(4096);
    shard.set_persist_producer(producer);
    shard.accounts.insert(0, Account::new(0, COLLATERAL));
    shard.accounts.insert(1, Account::new(1, COLLATERAL));
    set_prices(&mut shard);
    shard.process_fill(&FillEvent {
        seq: 1,
        symbol_id: 0,
        taker_user_id: 0,
        maker_user_id: 1,
        price: 10_000,
        qty: 100,
        taker_side: 0,
        timestamp_ns: 0,
    });
    (shard, consumer)
}

/// A shard booted from Postgres, as `run_node` builds one.
async fn booted_from(client: &Client) -> (RiskShard, Consumer<PersistEvent>) {
    let mut shard = RiskShard::new(shard_config());
    let (producer, consumer) = rtrb::RingBuffer::<PersistEvent>::new(4096);
    shard.set_persist_producer(producer);
    let state = load_from_postgres(client, 0, 1, 4).await.unwrap();
    shard.set_state(state);
    set_prices(&mut shard);
    (shard, consumer)
}

/// What the persist worker does with one drain: one transaction.
async fn flush(client: &mut Client, consumer: &mut Consumer<PersistEvent>) -> Vec<PersistEvent> {
    let mut events = Vec::new();
    while let Ok(event) = consumer.pop() {
        events.push(event);
    }
    flush_batch(client, 0, &events).await.unwrap();
    events
}

/// `(interval_id, payments)` of symbol 0's settlements, in event order.
fn symbol_zero_settlements(events: &[PersistEvent]) -> Vec<(u64, usize)> {
    let mut settled = Vec::new();
    for event in events {
        if let PersistEvent::FundingSettlement(settlement) = event {
            if settlement.symbol_id == 0 {
                settled.push((settlement.interval_id, settlement.payments.len()));
            }
        }
    }
    settled
}

async fn settled_intervals(client: &Client) -> Vec<i64> {
    let rows = client
        .query(
            "SELECT interval_id FROM funding_intervals \
             WHERE shard_id = 0 AND symbol_id = 0 ORDER BY interval_id",
            &[],
        )
        .await
        .unwrap();
    rows.iter().map(|row| row.get(0)).collect()
}

#[tokio::test]
#[ignore]
async fn restart_right_after_settlement_does_not_settle_again() {
    let (_pg, mut client) = migrated_pg().await;
    let (mut main, mut ring) = main_with_positions();
    flush(&mut client, &mut ring).await;
    main.maybe_settle_funding(3 * INTERVAL_SECS);
    let settled = flush(&mut client, &mut ring).await;
    assert_eq!(symbol_zero_settlements(&settled), vec![(3, 2)]);
    let collateral = main.accounts[&0].collateral;
    assert_eq!(collateral, COLLATERAL - PER_INTERVAL);

    let (mut restarted, mut ring) = booted_from(&client).await;
    assert_eq!(restarted.accounts[&0].collateral, collateral);
    assert_eq!(restarted.funding_settled[0], Some(3));
    restarted.maybe_settle_funding(3 * INTERVAL_SECS);
    let again = flush(&mut client, &mut ring).await;

    assert!(symbol_zero_settlements(&again).is_empty());
    assert_eq!(restarted.accounts[&0].collateral, collateral);
    assert_eq!(settled_intervals(&client).await, vec![3]);
    let rows = client
        .query("SELECT count(*) FROM funding", &[])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 2);
}

#[tokio::test]
#[ignore]
async fn restart_after_missed_intervals_settles_exactly_those_in_order() {
    let (_pg, mut client) = migrated_pg().await;
    let (mut main, mut ring) = main_with_positions();
    flush(&mut client, &mut ring).await;
    main.maybe_settle_funding(3 * INTERVAL_SECS);
    flush(&mut client, &mut ring).await;

    // Down through intervals 4, 5, 6; back up inside interval 7.
    let (mut restarted, mut ring) = booted_from(&client).await;
    restarted.maybe_settle_funding(7 * INTERVAL_SECS + 5);
    let caught_up = flush(&mut client, &mut ring).await;

    assert_eq!(
        symbol_zero_settlements(&caught_up),
        vec![(4, 2), (5, 2), (6, 2), (7, 2)]
    );
    assert_eq!(settled_intervals(&client).await, vec![3, 4, 5, 6, 7]);
    assert_eq!(
        restarted.accounts[&0].collateral,
        COLLATERAL - 5 * PER_INTERVAL
    );
    assert_eq!(
        restarted.accounts[&1].collateral,
        COLLATERAL + 5 * PER_INTERVAL
    );

    let (mut again, mut ring) = booted_from(&client).await;
    again.maybe_settle_funding(7 * INTERVAL_SECS + 9);
    let events = flush(&mut client, &mut ring).await;
    assert!(symbol_zero_settlements(&events).is_empty());
    assert_eq!(again.accounts[&0].collateral, COLLATERAL - 5 * PER_INTERVAL);
}

#[tokio::test]
#[ignore]
async fn funding_sums_to_zero_for_every_interval() {
    let (_pg, mut client) = migrated_pg().await;
    let (mut main, mut ring) = main_with_positions();
    flush(&mut client, &mut ring).await;
    main.maybe_settle_funding(3 * INTERVAL_SECS);
    flush(&mut client, &mut ring).await;
    let (mut restarted, mut ring) = booted_from(&client).await;
    restarted.maybe_settle_funding(7 * INTERVAL_SECS);
    flush(&mut client, &mut ring).await;

    let rows = client
        .query(
            "SELECT interval_id, sum(amount)::BIGINT, count(*) FROM funding \
             WHERE symbol_id = 0 GROUP BY interval_id ORDER BY interval_id",
            &[],
        )
        .await
        .unwrap();
    let intervals: Vec<i64> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(intervals, vec![3, 4, 5, 6, 7]);
    for row in &rows {
        assert_eq!(row.get::<_, i64>(1), 0, "interval {}", row.get::<_, i64>(0));
        assert_eq!(row.get::<_, i64>(2), 2);
    }
}

#[tokio::test]
#[ignore]
async fn warm_replica_does_not_double_settle_on_promotion() {
    let (_pg, mut client) = migrated_pg().await;
    let (mut main, mut main_ring) = main_with_positions();
    flush(&mut client, &mut main_ring).await;

    let (mut replica, mut replica_ring) = booted_from(&client).await;
    assert_eq!(replica.funding_settled[0], None);

    main.maybe_settle_funding(3 * INTERVAL_SECS);
    flush(&mut client, &mut main_ring).await;
    assert_eq!(main.accounts[&0].collateral, COLLATERAL - PER_INTERVAL);

    replica.follow_funding(&client).await.unwrap();
    assert_eq!(replica.funding_settled[0], Some(3));
    assert_eq!(replica.accounts[&0].collateral, COLLATERAL - PER_INTERVAL);
    assert_eq!(replica.accounts[&1].collateral, COLLATERAL + PER_INTERVAL);

    replica.maybe_settle_funding(3 * INTERVAL_SECS);
    replica.maybe_settle_funding(4 * INTERVAL_SECS);
    let events = flush(&mut client, &mut replica_ring).await;

    assert_eq!(symbol_zero_settlements(&events), vec![(4, 2)]);
    assert_eq!(settled_intervals(&client).await, vec![3, 4]);
    assert_eq!(
        replica.accounts[&0].collateral,
        COLLATERAL - 2 * PER_INTERVAL
    );
    let rows = client
        .query("SELECT collateral FROM accounts WHERE user_id = 0", &[])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), COLLATERAL - 2 * PER_INTERVAL);
}

#[tokio::test]
#[ignore]
async fn settling_an_interval_twice_rolls_back_the_whole_batch() {
    let (_pg, mut client) = migrated_pg().await;
    let (mut main, mut ring) = main_with_positions();
    flush(&mut client, &mut ring).await;
    main.maybe_settle_funding(3 * INTERVAL_SECS);
    let settled = flush(&mut client, &mut ring).await;
    let first = settled
        .into_iter()
        .find(|event| matches!(event, PersistEvent::FundingSettlement(s) if s.symbol_id == 0))
        .unwrap();

    let mut tampered = Account::new(0, 1);
    tampered.version = 99;
    let batch = [first, PersistEvent::Account(tampered)];
    let result = flush_batch(&mut client, 0, &batch).await;

    assert!(result.is_err(), "interval 3 is already recorded");
    assert_eq!(settled_intervals(&client).await, vec![3]);
    let rows = client
        .query("SELECT collateral FROM accounts WHERE user_id = 0", &[])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), COLLATERAL - PER_INTERVAL);
    let rows = client
        .query("SELECT count(*) FROM funding", &[])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 2);
}

#[tokio::test]
#[ignore]
async fn migration_008_keeps_payments_recorded_before_interval_ids() {
    let (_pg, client) = migrated_pg().await;
    client
        .batch_execute(
            "INSERT INTO funding (user_id, symbol_id, amount, rate, settlement_ts) \
             VALUES (1, 0, 5, 1, 28800), (1, 0, 5, 1, 28900);",
        )
        .await
        .unwrap();
    run_migrations(&client).await.unwrap();
    let rows = client
        .query(
            "SELECT count(*) FROM funding WHERE interval_id IS NULL",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 2);
}
