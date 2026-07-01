//! P0 tests for the orphan-freeze fix (bugs.md ORPHAN-FREEZE)
//! and the WAL-replay freeze rebuild path.
//!
//! The fix makes WAL `RECORD_ORDER_ACCEPTED` the sole authority
//! for *durable* freezes: `process_order` keeps the in-memory
//! freeze (pre-trade gate) but no longer write-behinds it to PG;
//! `confirm_freeze` (called when ME's OrderAccepted comes back)
//! is the only place a `FrozenInsert` is persisted. Recovery
//! loads freezes from PG (now only ME-confirmed ones) plus
//! replays the WAL, so a freeze ME never accepted can never
//! survive recovery.

use rsx_cast::WalWriter;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_risk::Account;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_risk::replay::ColdStartState;
use rsx_risk::replay::replay_from_wal;
use rsx_types::Qty;
use rustc_hash::FxHashMap;
use tempfile::TempDir;

fn default_config() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 2,
        max_symbols: 4,
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
            };
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    }
}

fn make_shard() -> RiskShard {
    RiskShard::new(default_config())
}

fn order(
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
    oid_lo: u64,
) -> OrderRequest {
    OrderRequest {
        seq: 1,
        user_id,
        symbol_id,
        price,
        qty,
        order_id_hi: 0,
        order_id_lo: oid_lo,
        timestamp_ns: 0,
        side: 0,
        tif: 0,
        reduce_only: false,
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    }
}

fn accepted(
    seq: u64,
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
    oid_lo: u64,
) -> OrderAcceptedRecord {
    OrderAcceptedRecord {
        seq,
        ts_ns: 0,
        user_id,
        symbol_id,
        order_id_hi: 0,
        order_id_lo: oid_lo,
        price,
        qty,
        side: 0,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        cid: [0u8; 20],
    }
}

/// Write a sequence of records to symbol `sid`'s WAL and flush.
/// Stream id == symbol id so `replay_from_wal` (which opens per
/// symbol) finds them.
fn write_wal<R, F>(wal_dir: &std::path::Path, sid: u32, recs: Vec<R>, mut set: F)
where
    R: rsx_cast::CastRecord + Copy,
    F: FnMut(&mut R, u64),
{
    let mut writer = WalWriter::new(sid, wal_dir, 64 * 1024 * 1024).unwrap();
    let mut seq = 1u64;
    for mut rec in recs {
        set(&mut rec, seq);
        let framed = writer.prepare(&mut rec).unwrap();
        writer.append_framed(&framed).unwrap();
        seq += 1;
    }
    writer.flush().unwrap();
}

// --- FM6: orphan-freeze does not survive recovery ---

#[test]
fn orphan_freeze_not_durable_without_order_accepted() {
    // An order that passed the pre-trade gate but ME never
    // confirmed must NOT leave a durable freeze. With the fix,
    // process_order does not persist the freeze; only
    // confirm_freeze (ME OrderAccepted) does. So a never-confirmed
    // order leaves nothing for PG/recovery to load.
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;

    // Observe the shard's durable-persist stream so the recovery
    // snapshot is DERIVED from what the shard actually persisted —
    // not a hand-picked empty map (which would make this circular).
    let (prod, mut cons) =
        rtrb::RingBuffer::<rsx_risk::persist::PersistEvent>::new(16);
    s.set_persist_producer(prod);

    // Order passes the pre-trade gate -> in-memory freeze.
    let o = order(0, 0, 10_000, 10, 42);
    let resp = s.process_order(&o);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
    assert!(s.frozen_for_user(0) > 0, "in-memory freeze for gate");

    // THE FIX: process_order persists nothing before ME confirms.
    // Build the PG recovery snapshot from the actual persist stream;
    // if the pre-send write-behind were reintroduced this map would
    // hold the orphan and the assertion below would fail.
    let mut fo = FxHashMap::default();
    while let Ok(ev) = cons.pop() {
        if let rsx_risk::persist::PersistEvent::FrozenInsert(f) = ev {
            let key = ((f.order_id_hi as u128) << 64)
                | f.order_id_lo as u128;
            fo.insert(key, (f.user_id, f.amount));
        }
    }
    assert!(
        fo.is_empty(),
        "ME never confirmed -> shard must have persisted no freeze",
    );

    // Recovery from that (genuinely empty) snapshot -> no orphan.
    let mut recovered = make_shard();
    recovered.set_state(ColdStartState {
        accounts: FxHashMap::default(),
        positions: FxHashMap::default(),
        tips: vec![0u64; 4],
        insurance_funds: FxHashMap::default(),
        frozen_orders: fo,
    });
    assert_eq!(
        recovered.frozen_for_user(0),
        0,
        "orphan freeze must not survive recovery",
    );
}

#[test]
fn confirmed_freeze_is_durable() {
    // Contrast: when ME confirms (confirm_freeze), the freeze is
    // persisted and thus survives recovery. We model the durable
    // record by feeding the confirmed amount into the cold-start
    // frozen_orders map (what the PG snapshot would contain).
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;

    // Wire a persist ring so confirm_freeze has somewhere to push.
    let (prod, mut cons) =
        rtrb::RingBuffer::<rsx_risk::persist::PersistEvent>::new(16);
    s.set_persist_producer(prod);

    let o = order(0, 0, 10_000, 10, 42);
    assert!(matches!(
        s.process_order(&o),
        OrderResponse::Accepted { .. }
    ));
    let amount = s.frozen_for_user(0);
    assert!(amount > 0);

    // process_order must NOT have persisted anything.
    assert!(
        cons.pop().is_err(),
        "process_order must not write-behind the freeze",
    );

    // ME confirms -> confirm_freeze persists the durable record.
    s.confirm_freeze(0, 0, 42, 0);
    let ev = cons.pop().expect("confirm_freeze must persist");
    let frozen = match ev {
        rsx_risk::persist::PersistEvent::FrozenInsert(f) => f,
        other => panic!("expected FrozenInsert, got {other:?}"),
    };
    assert_eq!(frozen.user_id, 0);
    assert_eq!(frozen.order_id_lo, 42);
    assert_eq!(frozen.amount, amount);

    // Recovery loads that durable record -> freeze present.
    let mut recovered = make_shard();
    let mut fo = FxHashMap::default();
    let key = (0u128 << 64) | frozen.order_id_lo as u128;
    fo.insert(key, (frozen.user_id, frozen.amount));
    recovered.set_state(ColdStartState {
        accounts: FxHashMap::default(),
        positions: FxHashMap::default(),
        tips: vec![0u64; 4],
        insurance_funds: FxHashMap::default(),
        frozen_orders: fo,
    });
    assert_eq!(recovered.frozen_for_user(0), amount);
}

// --- FM11: replay_freeze_order rebuild from WAL ---

#[test]
fn replay_order_accepted_rebuilds_freeze() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    // One OrderAccepted (reduce_only=0) on symbol 0 for user 0.
    write_wal(
        &wal_dir,
        0,
        vec![accepted(0, 0, 0, 10_000, 10, 42)],
        |r, seq| r.seq = seq,
    );

    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();

    // Expected freeze: IM + taker fee.
    // notional = 10000*10 = 100_000
    // im = 100_000 * 1000/10000 = 10_000
    // fee = 10000*10*5/10000 = 50
    assert_eq!(s.frozen_for_user(0), 10_050);
}

#[test]
fn replay_order_cancelled_releases_freeze() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let mut writer =
        WalWriter::new(0, &wal_dir, 64 * 1024 * 1024).unwrap();
    let mut acc = accepted(0, 0, 0, 10_000, 10, 42);
    let f = writer.prepare(&mut acc).unwrap();
    writer.append_framed(&f).unwrap();
    let mut canc = OrderCancelledRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 0,
        user_id: 0,
        order_id_hi: 0,
        order_id_lo: 42,
        remaining_qty: Qty(10),
        reason: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let f = writer.prepare(&mut canc).unwrap();
    writer.append_framed(&f).unwrap();
    writer.flush().unwrap();

    // Control: accepted-replay alone freezes a real amount, so the
    // 0 below proves the cancel RELEASED a freeze (not vacuous).
    {
        let cdir = tmp.path().join("ctrl");
        std::fs::create_dir_all(&cdir).unwrap();
        write_wal(&cdir, 0, vec![accepted(0, 0, 0, 10_000, 10, 42)], |r, seq| {
            r.seq = seq
        });
        let mut c = make_shard();
        c.accounts.insert(0, Account::new(0, 1_000_000_000));
        replay_from_wal(&mut c, &cdir, &[0]).unwrap();
        assert_eq!(c.frozen_for_user(0), 10_050, "control: accepted freezes");
    }

    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();
    assert_eq!(
        s.frozen_for_user(0),
        0,
        "cancel must release replayed freeze",
    );
}

#[test]
fn replay_order_failed_releases_freeze() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let mut writer =
        WalWriter::new(0, &wal_dir, 64 * 1024 * 1024).unwrap();
    let mut acc = accepted(0, 0, 0, 10_000, 10, 42);
    let f = writer.prepare(&mut acc).unwrap();
    writer.append_framed(&f).unwrap();
    let mut fail = OrderFailedRecord {
        seq: 0,
        ts_ns: 0,
        user_id: 0,
        _pad0: 0,
        order_id_hi: 0,
        order_id_lo: 42,
        reason: 0,
        _pad: [0; 23],
    };
    let f = writer.prepare(&mut fail).unwrap();
    writer.append_framed(&f).unwrap();
    writer.flush().unwrap();

    // Control: accepted-replay alone freezes a real amount, so the
    // 0 below proves order-failed RELEASED a freeze (not vacuous).
    {
        let cdir = tmp.path().join("ctrl");
        std::fs::create_dir_all(&cdir).unwrap();
        write_wal(&cdir, 0, vec![accepted(0, 0, 0, 10_000, 10, 42)], |r, seq| {
            r.seq = seq
        });
        let mut c = make_shard();
        c.accounts.insert(0, Account::new(0, 1_000_000_000));
        replay_from_wal(&mut c, &cdir, &[0]).unwrap();
        assert_eq!(c.frozen_for_user(0), 10_050, "control: accepted freezes");
    }

    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();
    assert_eq!(
        s.frozen_for_user(0),
        0,
        "order-failed must release replayed freeze",
    );
}

// --- FM13a: exactly-once under retry (duplicate order_id) ---

#[test]
fn duplicate_order_id_freezes_once() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;

    let o = order(0, 0, 10_000, 10, 42);
    assert!(matches!(
        s.process_order(&o),
        OrderResponse::Accepted { .. }
    ));
    let once = s.frozen_for_user(0);
    assert!(once > 0);

    // Same order_id fed again (retry / duplicate delivery).
    assert!(matches!(
        s.process_order(&o),
        OrderResponse::Accepted { .. }
    ));
    assert_eq!(
        s.frozen_for_user(0),
        once,
        "duplicate order_id must not double-freeze",
    );
}

#[test]
fn duplicate_fill_seq_positions_once() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    // OrderAccepted then a Fill (same symbol). Replaying twice
    // must apply the fill once (seq dedup) and the freeze once.
    let mut writer =
        WalWriter::new(0, &wal_dir, 64 * 1024 * 1024).unwrap();
    let mut acc = accepted(0, 0, 0, 10_000, 10, 42);
    let f = writer.prepare(&mut acc).unwrap();
    writer.append_framed(&f).unwrap();
    let mut fill = rsx_messages::FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 0,
        taker_user_id: 0,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 42,
        maker_order_id_hi: 0,
        maker_order_id_lo: 99,
        price: rsx_types::Price(10_000),
        qty: Qty(10),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    };
    let f = writer.prepare(&mut fill).unwrap();
    writer.append_framed(&f).unwrap();
    writer.flush().unwrap();

    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();
    assert_eq!(s.positions[&(0, 0)].long_qty, 10);
    assert_eq!(s.fills_processed, 1);

    // Second pass over the same WAL (tip already at 2) -> no-op.
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();
    assert_eq!(s.positions[&(0, 0)].long_qty, 10);
    assert_eq!(s.fills_processed, 1);
}

// --- FM14: double-replay idempotency ---

#[test]
fn double_replay_identical_state() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    // accepted(42) + accepted(43) + done(42).
    let mut writer =
        WalWriter::new(0, &wal_dir, 64 * 1024 * 1024).unwrap();
    let mut a1 = accepted(0, 0, 0, 10_000, 10, 42);
    let f = writer.prepare(&mut a1).unwrap();
    writer.append_framed(&f).unwrap();
    let mut a2 = accepted(0, 0, 0, 10_000, 7, 43);
    let f = writer.prepare(&mut a2).unwrap();
    writer.append_framed(&f).unwrap();
    let mut done = OrderDoneRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 0,
        user_id: 0,
        order_id_hi: 0,
        order_id_lo: 42,
        filled_qty: Qty(10),
        remaining_qty: Qty(0),
        final_status: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    let f = writer.prepare(&mut done).unwrap();
    writer.append_framed(&f).unwrap();
    writer.flush().unwrap();

    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();

    let frozen_after_1 = s.frozen_for_user(0);
    let tips_after_1 = s.tips.clone();
    // Only order 43 should remain frozen (42 was done).
    // notional 7*10000=70000; im=7000; fee=7*10000*5/10000=35
    assert_eq!(frozen_after_1, 7_035);

    // Second full replay over the same records.
    replay_from_wal(&mut s, &wal_dir, &[0]).unwrap();
    assert_eq!(
        s.frozen_for_user(0),
        frozen_after_1,
        "double replay must not change frozen",
    );
    assert_eq!(
        s.tips, tips_after_1,
        "double replay must not change tips",
    );
}
