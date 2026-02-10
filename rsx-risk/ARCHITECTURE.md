# rsx-risk Architecture

Risk engine process. One instance per user shard. Pre-trade
margin checks, position tracking, funding, liquidation,
insurance fund, and replication. See `specs/v1/RISK.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: main loop, CMP pump, replica mode, promotion |
| `shard.rs` | `RiskShard` -- core state machine, `run_once()` |
| `types.rs` | `FillEvent`, `OrderRequest`, `BboUpdate`, `RejectReason` |
| `account.rs` | `Account` struct and balance operations |
| `position.rs` | `Position` struct, fill application |
| `margin.rs` | `PortfolioMargin`, margin requirement calculation |
| `price.rs` | `IndexPrice`, mark price updates |
| `funding.rs` | Funding rate computation and payment application |
| `liquidation.rs` | Liquidation detection and order generation |
| `insurance.rs` | `InsuranceFund`, socialized loss |
| `persist.rs` | Async Postgres persistence via SPSC ring |
| `replay.rs` | Cold start from Postgres + WAL replay |
| `schema.rs` | Postgres table creation |
| `lease.rs` | `AdvisoryLease` for single-writer guarantee |
| `replica.rs` | `ReplicaState`, fill buffering, promotion |
| `rings.rs` | `ShardRings`, `OrderResponse` |
| `config.rs` | `ShardConfig`, `ReplicationConfig` |
| `risk_utils.rs` | Fee calculation |

## Key Types

- `RiskShard` -- central state: accounts, positions, margin
  engine, index/mark prices, tips, insurance funds,
  liquidation engine
- `Account` -- balance, frozen margin, equity
- `Position` -- per (user, symbol): qty, entry price,
  realized PnL
- `PortfolioMargin` -- margin calculation with `SymbolRiskParams`
- `InsuranceFund` -- per-symbol insurance balance
- `LiquidationEngine` -- liquidation detection and order gen
- `AdvisoryLease` -- Postgres advisory lock for single-writer
- `ReplicaState` -- buffered fills and tip tracking for failover

## Main Loop

Single-threaded busy-spin on dedicated core. Priority order:

```
loop {
    1. Fills from all MEs     (highest priority)
    2. Orders from gateway    (never skip)
    3. Mark prices from DXS   (update price feeds)
    4. BBO updates             (trigger margin recalc)
    5. Funding settlement      (every 8h)
    6. Liquidation processing  (if triggered)
    7. Lease renewal           (every ~1s)
}
```

## Position Tracking

In-memory `FxHashMap<(user_id, symbol_id), Position>`:

```rust
struct Position {
    long_qty: i64,
    short_qty: i64,
    long_entry_cost: i64,   // sum(price * qty)
    short_entry_cost: i64,
    realized_pnl: i64,
    last_fill_seq: u64,
    version: u64,           // CAS for PG upsert
}
```

Position flip (long->short): close old at fill price
(realize PnL), open new with entry = fill price. Two-step
in `apply_fill`.

## Margin Calculation

```
equity = collateral + sum(unrealized_pnl)
initial_margin = sum(|net_qty| * mark_price * im_rate)
maint_margin = sum(|net_qty| * mark_price * mm_rate)
available = equity - initial_margin - frozen_margin
```

Recalculated on every price tick for all users with exposure
in the updated symbol. Exposure index (`Vec<Vec<u32>>`)
tracks which users have positions per symbol.

## Pre-Trade Risk Check

```
1. Full portfolio margin recalc with latest mark prices
2. Calculate order initial margin + worst-case taker fee
3. Reject if available < margin_needed + fee_reserve
4. Freeze margin: account.frozen_margin += margin_needed
5. Route order to matching engine
```

On ORDER_DONE: release frozen margin. Frozen margin tracked
per-order in memory (not persisted; lost on restart).

## Liquidation

When `equity < maint_margin`:
1. Enqueue user for liquidation
2. Each round: close largest position with market order
3. Escalation: increase slippage tolerance per round
4. Insurance fund absorbs losses beyond bankruptcy price
5. Socialized loss if insurance fund exhausted

## Funding

Every 8 hours (UTC 00:00, 08:00, 16:00):
```
premium = (mark_price - index_price) / index_price
funding_payment = position_qty * mark_price * rate
```
Long pays short when rate > 0. Zero-sum per symbol per
interval. Idempotency key: `interval_id = epoch_secs / 28800`.

## Persistence

Write-behind on separate thread, 10ms flush:
- Positions: batched UPSERT (advisory lock = single writer)
- Fills: COPY binary (bulk insert)
- Tips: batched UPSERT per (instance_id, symbol_id)
- `synchronous_commit = on`
- Backpressure: PG lag > 100ms stalls hot path

## Replication and Failover

**Main:** acquire advisory lock -> load from PG -> DXS replay
from tip+1 -> CaughtUp on all streams -> go live.

**Replica:** try lock (fails) -> buffer fills from MEs ->
poll `pg_try_advisory_lock` every ~500ms -> if acquired:
apply buffered fills up to last tip -> promote.

Data loss bound: 10ms single crash, 100ms dual crash.

## Deduplication

- Fills: `seq <= tips[symbol_id]` -> skip (idempotent replay)
- Tips: monotonic, never decrease
- PG positions: version field + UPSERT (defensive)

## Performance Targets

| Path | Target |
|------|--------|
| Fill processing | <1us |
| Pre-trade check | <5us |
| Per-tick margin recalc | <10us/user |
| BBO -> index price | <100ns |
| Postgres flush | every 10ms |
| Failover detection | ~500ms |
