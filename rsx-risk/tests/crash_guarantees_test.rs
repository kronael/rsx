//! Failover / crash-guarantee integration tests: the replica,
//! failover, and write-behind durability contracts of the risk
//! shard, against a real Postgres (testcontainers) and a real ME
//! WAL on disk.
//!
//! - `failover_handoff_no_lost_fill_no_double_fill` — invariant #10
//!   (at most one main per shard) end-to-end: main A persists live
//!   fills behind the advisory lock, warm B cold-loads PG state and
//!   catches up from the ME WAL, A crashes, B promotes and owns
//!   persistence. Every fill lands in the final position exactly
//!   once (invariant #4) and tips only advance (invariant #5).
//! - `crash_recovery_equals_full_application` — write-behind
//!   durability: crash with fills applied but never flushed; PG
//!   snapshot + WAL replay from tip+1 rebuilds exactly the state a
//!   crash-free shard would hold, and a second replay is a no-op.
//! - `flush_batch_money_mutations_all_or_nothing` — a poisoned
//!   persist batch (position + account + insurance + liquidation +
//!   duplicate fill) rolls back atomically: statements that already
//!   executed inside the transaction do not survive, and the same
//!   client recovers on retry (the persist worker's retry-loop
//!   assumption).
//! - `funding_settlement_zero_sum_in_postgres` — invariant #9 at the
//!   persistence layer: settled funding rows sum to zero per symbol
//!   and mirror the in-RAM collateral deltas exactly.
//!
//! All need Docker; `#[ignore]`, run under `make integration`.

mod common;

use crate::common::connect;
use crate::common::replay_wal_dir;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::persist::flush_batch;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::persist::PersistEvent;
use rsx_risk::persist::StopMode;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::schema::run_migrations;
use rsx_risk::Account;
use rsx_risk::FillEvent;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::Position;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_types::Price;
use rsx_types::Qty;
use rtrb::RingBuffer;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_postgres::Client;

const SYMBOL: u32 = 1;
const TAKER: u32 = 4;
const MAKER: u32 = 8;
const FILL_PRICE: i64 = 10_000;
const FILL_QTY: i64 = 100;
const SEED_COLLATERAL: i64 = 1_000_000;
const LEASE_SHARD: u32 = 210;

fn test_config() -> ShardConfig {
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
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    }
}

fn seed_accounts(shard: &mut RiskShard) {
    for user_id in [TAKER, MAKER] {
        shard.accounts.insert(
            user_id,
            Account {
                user_id,
                collateral: SEED_COLLATERAL,
                version: 0,
            },
        );
    }
}

/// The live-side view of fill `seq`, exactly matching `wal_fill(seq)`.
fn fill_event(seq: u64) -> FillEvent {
    FillEvent {
        seq,
        symbol_id: SYMBOL,
        taker_user_id: TAKER,
        maker_user_id: MAKER,
        price: FILL_PRICE,
        qty: FILL_QTY,
        taker_side: 0,
        timestamp_ns: 1_000 + seq,
    }
}

/// The wire/WAL view of fill `seq`, exactly matching `fill_event(seq)`.
fn wal_fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000 + seq,
        symbol_id: SYMBOL,
        taker_user_id: TAKER,
        maker_user_id: MAKER,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
        price: Price(FILL_PRICE),
        qty: Qty(FILL_QTY),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

fn append_fills(writer: &mut WalWriter, seqs: std::ops::RangeInclusive<u64>) {
    for seq in seqs {
        let mut record = wal_fill(seq);
        let framed = writer.prepare(&mut record).expect("frame fill");
        writer.append_framed(&framed).expect("append fill");
    }
    writer.flush().expect("flush wal");
}

/// Poll the `tips` table (instance 0) until `expected_seq` appears.
async fn wait_for_tip(client: &Client, symbol_id: u32, expected_seq: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            panic!(
                "tip seq>={expected_seq} for sym {symbol_id} \
                 not observed within deadline",
            );
        }
        let rows = client
            .query(
                "SELECT last_seq FROM tips \
                 WHERE instance_id = 0 AND symbol_id = $1",
                &[&(symbol_id as i32)],
            )
            .await;
        if let Ok(rows) = rows {
            if let Some(row) = rows.first() {
                let seq: i64 = row.get(0);
                if seq as u64 >= expected_seq {
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn assert_positions_equal(actual: &RiskShard, expected: &RiskShard, user_id: u32) {
    let a = actual
        .positions
        .get(&(user_id, SYMBOL))
        .unwrap_or_else(|| panic!("actual shard has no position for user {user_id}"));
    let e = expected
        .positions
        .get(&(user_id, SYMBOL))
        .unwrap_or_else(|| panic!("expected shard has no position for user {user_id}"));
    assert_eq!(
        (
            a.long_qty,
            a.short_qty,
            a.long_entry_cost,
            a.short_entry_cost,
            a.realized_pnl,
            a.last_fill_seq,
        ),
        (
            e.long_qty,
            e.short_qty,
            e.long_entry_cost,
            e.short_entry_cost,
            e.realized_pnl,
            e.last_fill_seq,
        ),
        "position mismatch for user {user_id}",
    );
}

/// Invariant #10 end-to-end: main A applies + persists fills behind
/// the advisory lock while warm B (cold-loaded from A's PG writes)
/// cannot promote; A crashes with fills 6..=8 in the ME WAL that A
/// never persisted; B replays exactly those from tip+1 (no
/// double-apply of 1..=5 — tip dedup, invariant #5), promotes, and
/// persists. Final PG state = every fill applied exactly once
/// (invariant #4), verified against an independent golden shard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn failover_handoff_no_lost_fill_no_double_fill() {
    let (_pg, port) = common::start_postgres().await;
    let verify = connect(port).await;
    run_migrations(&verify).await.expect("migrations");

    // Golden: what a crash-free shard holds after fills 1..=9.
    let mut golden = RiskShard::new(test_config());
    seed_accounts(&mut golden);
    for seq in 1..=9 {
        golden.process_fill(&fill_event(seq));
    }

    // The ME's WAL. Fills 1..=5 exist while A is the live main.
    let tmp = TempDir::new().expect("tempdir");
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).expect("wal dir");
    let mut writer = WalWriter::new(SYMBOL, &wal_dir, 64 * 1024 * 1024).expect("wal writer");
    append_fills(&mut writer, 1..=5);
    assert_eq!(writer.last_seq(), 5);

    // Main A: holds the lock, applies fills live, persists.
    let lease_client_a = connect(port).await;
    let mut lease_a = AdvisoryLease::new(LEASE_SHARD);
    assert!(lease_a.try_acquire(&lease_client_a).await.expect("A lock"));

    let mut main_a = RiskShard::new(test_config());
    seed_accounts(&mut main_a);
    let (producer_a, consumer_a) = RingBuffer::new(1024);
    main_a.set_persist_producer(producer_a);
    let worker_client_a = connect(port).await;
    let shutdown_a = Arc::new(AtomicU8::new(StopMode::Run as u8));
    let worker_a = tokio::spawn(run_persist_worker_with_shutdown(
        consumer_a,
        worker_client_a,
        0,
        Some(shutdown_a.clone()),
        None,
        None,
    ));
    for seq in 1..=5 {
        main_a.process_fill(&fill_event(seq));
    }
    wait_for_tip(&verify, SYMBOL, 5).await;

    // Warm B: cold-loads A's persisted state (the real warm-candidate
    // boot path), then must NOT be able to promote while A lives.
    let state = load_from_postgres(&verify, 0, 1, 4)
        .await
        .expect("cold load");
    assert_eq!(state.tips[SYMBOL as usize], 5);
    let mut warm_b = RiskShard::new(test_config());
    warm_b.set_state(state);
    assert_eq!(
        warm_b
            .positions
            .get(&(TAKER, SYMBOL))
            .expect("loaded position")
            .long_qty,
        5 * FILL_QTY,
    );
    let lease_client_b = connect(port).await;
    let mut lease_b = AdvisoryLease::new(LEASE_SHARD);
    assert!(
        !lease_b.try_acquire(&lease_client_b).await.expect("B try"),
        "B must not promote while A holds the lock",
    );

    // A crashes: worker stops, lease session drops -> PG releases the
    // lock. The ME keeps matching: fills 6..=8 land in the WAL with no
    // live main to persist them.
    shutdown_a.store(StopMode::FlushAndStop as u8, Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(10), worker_a)
        .await
        .expect("worker A stop timeout")
        .expect("worker A join");
    drop(lease_client_a);
    append_fills(&mut writer, 6..=8);
    assert_eq!(writer.last_seq(), 8);

    // B catches up from its own tip: only 6..=8 APPLY. The reader
    // re-reads the whole segment (and `replay_wal_dir` counts every
    // fill record read), but the tip dedup in `process_fill` makes
    // 1..=5 no-ops — the state, not the read count, carries the
    // invariant: position moves 5 -> 8 fills, tip 5 -> 8, never back.
    replay_wal_dir(&mut warm_b, &wal_dir, &[SYMBOL]).expect("catchup");
    assert_eq!(warm_b.tips[SYMBOL as usize], 8);
    assert_eq!(
        warm_b
            .positions
            .get(&(TAKER, SYMBOL))
            .expect("caught-up position")
            .long_qty,
        8 * FILL_QTY,
        "catchup applies 6..=8 once and re-applies nothing",
    );

    // B promotes once PG drops A's session lock.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if lease_b
            .try_acquire(&lease_client_b)
            .await
            .expect("B acquire")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "B could not promote after A's crash",
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Promoted B owns persistence: one live fill flows through B's
    // ring and flushes B's cumulative state (fills 1..=9, each once).
    let (producer_b, consumer_b) = RingBuffer::new(1024);
    warm_b.set_persist_producer(producer_b);
    let worker_client_b = connect(port).await;
    let shutdown_b = Arc::new(AtomicU8::new(StopMode::Run as u8));
    let worker_b = tokio::spawn(run_persist_worker_with_shutdown(
        consumer_b,
        worker_client_b,
        0,
        Some(shutdown_b.clone()),
        None,
        None,
    ));
    append_fills(&mut writer, 9..=9);
    warm_b.process_fill(&fill_event(9));
    wait_for_tip(&verify, SYMBOL, 9).await;

    // PG position/account = golden (every fill exactly once).
    let golden_taker = golden
        .positions
        .get(&(TAKER, SYMBOL))
        .expect("golden taker");
    let row = verify
        .query_one(
            "SELECT long_qty, short_qty, realized_pnl, last_fill_seq \
             FROM positions WHERE user_id = $1 AND symbol_id = $2",
            &[&(TAKER as i32), &(SYMBOL as i32)],
        )
        .await
        .expect("taker position row");
    assert_eq!(row.get::<_, i64>(0), golden_taker.long_qty);
    assert_eq!(row.get::<_, i64>(1), golden_taker.short_qty);
    assert_eq!(row.get::<_, i64>(2), golden_taker.realized_pnl);
    assert_eq!(row.get::<_, i64>(3) as u64, golden_taker.last_fill_seq);

    let golden_maker = golden
        .positions
        .get(&(MAKER, SYMBOL))
        .expect("golden maker");
    let row = verify
        .query_one(
            "SELECT short_qty FROM positions \
             WHERE user_id = $1 AND symbol_id = $2",
            &[&(MAKER as i32), &(SYMBOL as i32)],
        )
        .await
        .expect("maker position row");
    assert_eq!(row.get::<_, i64>(0), golden_maker.short_qty);

    for user_id in [TAKER, MAKER] {
        let golden_collateral = golden.accounts[&user_id].collateral;
        let row = verify
            .query_one(
                "SELECT collateral FROM accounts WHERE user_id = $1",
                &[&(user_id as i32)],
            )
            .await
            .expect("account row");
        assert_eq!(
            row.get::<_, i64>(0),
            golden_collateral,
            "fee continuity across the handoff for user {user_id}",
        );
    }

    // Fill audit rows: A live-inserted 1..=5, B live-inserted 9.
    // Replayed fills (6..=8) restore state but do not re-insert audit
    // rows — the ME WAL (archived by rsx-recorder) is the
    // authoritative fill log; PG `fills` records live processing only.
    let rows = verify
        .query(
            "SELECT seq FROM fills WHERE symbol_id = $1 ORDER BY seq",
            &[&(SYMBOL as i32)],
        )
        .await
        .expect("fills rows");
    let seqs: Vec<i64> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 9]);

    // Idempotency coda: a second replay over the same WAL re-reads
    // records but applies nothing — state stays equal to golden.
    replay_wal_dir(&mut warm_b, &wal_dir, &[SYMBOL]).expect("re-replay");
    assert_positions_equal(&warm_b, &golden, TAKER);
    assert_positions_equal(&warm_b, &golden, MAKER);
    assert_eq!(warm_b.tips[SYMBOL as usize], 9, "tip never decreases");

    shutdown_b.store(StopMode::FlushAndStop as u8, Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(10), worker_b)
        .await
        .expect("worker B stop timeout")
        .expect("worker B join");
}

/// Write-behind durability: main A crashes having applied fills
/// 1..=6 (flushed) while the ME WAL holds 1..=10. Recovery =
/// cold-load the PG snapshot + replay the WAL from tip+1. The
/// recovered shard must equal a crash-free golden shard on
/// positions, accounts, and tips — and a second replay must be a
/// no-op (invariants #4 and #5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn crash_recovery_equals_full_application() {
    let (_pg, port) = common::start_postgres().await;
    let verify = connect(port).await;
    run_migrations(&verify).await.expect("migrations");

    let tmp = TempDir::new().expect("tempdir");
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).expect("wal dir");
    let mut writer = WalWriter::new(SYMBOL, &wal_dir, 64 * 1024 * 1024).expect("wal writer");
    append_fills(&mut writer, 1..=10);

    let mut golden = RiskShard::new(test_config());
    seed_accounts(&mut golden);
    for seq in 1..=10 {
        golden.process_fill(&fill_event(seq));
    }

    // Main A: persists through fill 6, then crashes. Fills 7..=10
    // exist only in the ME WAL.
    let mut main_a = RiskShard::new(test_config());
    seed_accounts(&mut main_a);
    let (producer_a, consumer_a) = RingBuffer::new(1024);
    main_a.set_persist_producer(producer_a);
    let worker_client = connect(port).await;
    let shutdown = Arc::new(AtomicU8::new(StopMode::Run as u8));
    let worker = tokio::spawn(run_persist_worker_with_shutdown(
        consumer_a,
        worker_client,
        0,
        Some(shutdown.clone()),
        None,
        None,
    ));
    for seq in 1..=6 {
        main_a.process_fill(&fill_event(seq));
    }
    wait_for_tip(&verify, SYMBOL, 6).await;
    shutdown.store(StopMode::FlushAndStop as u8, Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .expect("worker stop timeout")
        .expect("worker join");
    drop(main_a);

    // Recovery: PG snapshot at tip 6 + WAL replay of 7..=10.
    let state = load_from_postgres(&verify, 0, 1, 4)
        .await
        .expect("cold load");
    assert_eq!(state.tips[SYMBOL as usize], 6);
    let mut recovered = RiskShard::new(test_config());
    recovered.set_state(state);
    assert_eq!(
        recovered
            .positions
            .get(&(TAKER, SYMBOL))
            .expect("partial position")
            .last_fill_seq,
        6,
        "pre-replay state is the partial snapshot",
    );
    // The reader re-reads the segment from its start; the tip dedup
    // in `process_fill` makes 1..=6 no-ops, so only 7..=10 APPLY —
    // the equality assertions below are the proof.
    replay_wal_dir(&mut recovered, &wal_dir, &[SYMBOL]).expect("replay");

    assert_positions_equal(&recovered, &golden, TAKER);
    assert_positions_equal(&recovered, &golden, MAKER);
    assert_eq!(
        recovered.tips[SYMBOL as usize],
        golden.tips[SYMBOL as usize]
    );
    for user_id in [TAKER, MAKER] {
        assert_eq!(
            recovered.accounts[&user_id].collateral, golden.accounts[&user_id].collateral,
            "collateral (fees) must survive recovery for user {user_id}",
        );
    }

    // Replaying the same range again applies nothing (tip dedup).
    replay_wal_dir(&mut recovered, &wal_dir, &[SYMBOL]).expect("re-replay");
    assert_positions_equal(&recovered, &golden, TAKER);
    assert_positions_equal(&recovered, &golden, MAKER);
    assert_eq!(
        recovered.tips[SYMBOL as usize], golden.tips[SYMBOL as usize],
        "tip never decreases",
    );
}

fn persist_fill(seq: u64) -> rsx_risk::persist::FillRecord {
    rsx_risk::persist::FillRecord {
        symbol_id: SYMBOL,
        taker_user_id: TAKER,
        maker_user_id: MAKER,
        price: FILL_PRICE,
        qty: FILL_QTY,
        taker_fee: 5,
        maker_fee: -1,
        taker_side: 0,
        seq,
        timestamp_ns: 1,
    }
}

/// A crash (or any error) inside one persist transaction must not
/// partially apply money mutations. The poisoned batch fails on its
/// LAST statement group (duplicate settled funding interval ->
/// primary-key violation on funding_intervals), after
/// positions/accounts/insurance already
/// executed inside the transaction — the rollback must erase them
/// all. The same client then lands a corrected batch, mirroring the
/// persist worker's retry loop.
#[tokio::test]
#[ignore = "requires docker; pg testcontainer"]
async fn flush_batch_money_mutations_all_or_nothing() {
    let (_pg, port) = common::start_postgres().await;
    let mut client = connect(port).await;
    run_migrations(&client).await.expect("migrations");

    let settlement = || {
        PersistEvent::FundingSettlement(rsx_risk::persist::FundingSettlement {
            symbol_id: SYMBOL,
            interval_id: 5,
            payments: Vec::new(),
            accounts: Vec::new(),
        })
    };
    flush_batch(
        &mut client,
        0,
        &[PersistEvent::Fill(persist_fill(42)), settlement()],
        None,
    )
    .await
    .expect("seed fill and settled interval");

    let mut victim = Position::new(7, SYMBOL);
    victim.long_qty = FILL_QTY;
    victim.long_entry_cost = FILL_PRICE * FILL_QTY;
    victim.version = 1;
    let poisoned = vec![
        PersistEvent::Position(victim.clone()),
        PersistEvent::Account(Account {
            user_id: 7,
            collateral: 555,
            version: 1,
        }),
        PersistEvent::InsuranceFund(rsx_risk::insurance::InsuranceFund::new(SYMBOL, 999)),
        PersistEvent::Liquidation(rsx_risk::persist::LiquidationRecord {
            user_id: 7,
            symbol_id: SYMBOL,
            round: 1,
            side: 0,
            price: FILL_PRICE,
            qty: FILL_QTY,
            slippage_bps: 10,
            status: 1,
            timestamp_ns: 1,
        }),
        PersistEvent::Tip {
            symbol_id: SYMBOL,
            seq: 77,
        },
        // Interval already settled -> the transaction must abort.
        settlement(),
    ];
    let result = flush_batch(&mut client, 0, &poisoned, None).await;
    assert!(
        result.is_err(),
        "a re-settled interval must abort the batch"
    );

    for (table, what) in [
        ("positions", "position"),
        ("accounts", "account"),
        ("insurance_fund", "insurance fund"),
        ("liquidations", "liquidation"),
        ("tips", "tip"),
    ] {
        let row = client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .expect("count");
        assert_eq!(
            row.get::<_, i64>(0),
            0,
            "{what} from the aborted batch must not survive the rollback",
        );
    }
    let row = client
        .query_one("SELECT count(*) FROM fills", &[])
        .await
        .expect("fills count");
    assert_eq!(row.get::<_, i64>(0), 1, "only the seeded fill remains");

    // Same client, corrected batch (no duplicate): lands completely.
    let good: Vec<PersistEvent> = poisoned[..5].to_vec();
    flush_batch(&mut client, 0, &good, None)
        .await
        .expect("corrected batch");
    for table in ["positions", "accounts", "insurance_fund", "liquidations"] {
        let row = client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .expect("count");
        assert_eq!(row.get::<_, i64>(0), 1, "{table} row lands on retry");
    }
    let row = client
        .query_one(
            "SELECT last_seq FROM tips \
             WHERE instance_id = 0 AND symbol_id = $1",
            &[&(SYMBOL as i32)],
        )
        .await
        .expect("tip row");
    assert_eq!(row.get::<_, i64>(0), 77);
}

/// Invariant #9 through the persistence layer: settle funding across
/// three users with opposing exposure, flush the settlement events,
/// and verify in SQL that the payments sum to exactly zero for the
/// symbol and mirror the in-RAM collateral deltas one-for-one.
#[tokio::test]
#[ignore = "requires docker; pg testcontainer"]
async fn funding_settlement_zero_sum_in_postgres() {
    let (_pg, port) = common::start_postgres().await;
    let mut client = connect(port).await;
    run_migrations(&client).await.expect("migrations");

    let mut shard = RiskShard::new(test_config());
    let users = [1u32, 2, 3];
    for user_id in users {
        shard.accounts.insert(
            user_id,
            Account {
                user_id,
                collateral: SEED_COLLATERAL,
                version: 0,
            },
        );
    }
    // User 1 long 100; users 2 and 3 short 60/40 on symbol 0.
    shard.process_fill(&FillEvent {
        seq: 1,
        symbol_id: 0,
        taker_user_id: 1,
        maker_user_id: 2,
        price: FILL_PRICE,
        qty: 60,
        taker_side: 0,
        timestamp_ns: 1,
    });
    shard.process_fill(&FillEvent {
        seq: 2,
        symbol_id: 0,
        taker_user_id: 1,
        maker_user_id: 3,
        price: FILL_PRICE,
        qty: 40,
        taker_side: 0,
        timestamp_ns: 2,
    });
    shard.mark_prices[0] = 10_100;
    shard.index_prices[0].price = 10_000;
    shard.index_prices[0].valid = true;

    let before: Vec<i64> = users.iter().map(|u| shard.accounts[u].collateral).collect();

    // Attach the ring only now so it carries settlement events alone.
    let (producer, mut consumer) = RingBuffer::new(256);
    shard.set_persist_producer(producer);
    shard.maybe_settle_funding(28_800);

    let mut events = Vec::new();
    while let Ok(event) = consumer.pop() {
        events.push(event);
    }
    assert!(!events.is_empty(), "settlement must emit persist events");
    flush_batch(&mut client, 0, &events, None)
        .await
        .expect("flush");

    let row = client
        .query_one(
            "SELECT COALESCE(SUM(amount), 0)::BIGINT \
             FROM funding WHERE symbol_id = 0",
            &[],
        )
        .await
        .expect("sum");
    assert_eq!(
        row.get::<_, i64>(0),
        0,
        "funding must be zero-sum (invariant #9)"
    );

    let rows = client
        .query(
            "SELECT user_id, amount FROM funding \
             WHERE symbol_id = 0 ORDER BY user_id",
            &[],
        )
        .await
        .expect("rows");
    assert_eq!(rows.len(), users.len(), "one payment row per exposed user");
    let mut nonzero = 0;
    for row in &rows {
        let user_id = row.get::<_, i32>(0) as u32;
        let amount: i64 = row.get(1);
        let idx = users
            .iter()
            .position(|u| *u == user_id)
            .expect("known user");
        let delta = shard.accounts[&user_id].collateral - before[idx];
        assert_eq!(
            amount, -delta,
            "persisted payment must mirror the RAM collateral delta \
             for user {user_id}",
        );
        if amount != 0 {
            nonzero += 1;
        }
    }
    assert!(nonzero >= 2, "mark != index must move real money");
}
