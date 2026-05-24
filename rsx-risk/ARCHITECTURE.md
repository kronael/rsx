# rsx-risk Architecture

Risk engine process. One instance per user shard. Pre-trade
margin checks, position tracking, funding, liquidation,
insurance fund, and main/replica replication. Canonical
full-tile arrangement per `specs/2/45-tiles.md` §3.2.

Specs: `specs/2/28-risk.md`, `specs/2/45-tiles.md` §3.2.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: ring construction, casting pump, replica mode, promotion |
| `shard.rs` | `RiskShard` — core state machine, `process_order()`, fill apply |
| `types.rs` | `FillEvent`, `OrderRequest`, `BboUpdate`, `RejectReason` |
| `account.rs` | `Account` struct and balance operations |
| `position.rs` | `Position` struct, fill application, long/short flip |
| `margin.rs` | `PortfolioMargin`, `check_order()` pre-trade gate |
| `price.rs` | `IndexPrice`, mark price updates |
| `funding.rs` | Funding rate computation and payment application |
| `liquidation.rs` | `LiquidationEngine`, detection and order generation |
| `insurance.rs` | `InsuranceFund`, socialized loss |
| `persist.rs` | Async Postgres persistence via SPSC ring (`PersistEvent`) |
| `replay.rs` | Cold start from Postgres + WAL replay |
| `schema.rs` | Postgres table creation |
| `lease.rs` | `AdvisoryLease` — Postgres advisory lock for single-writer |
| `replica.rs` | `ReplicaState`, fill buffering, promotion |
| `rings.rs` | `ShardRings`, `OrderResponse`, `MarkPriceUpdate` |
| `config.rs` | `ShardConfig`, `ReplicationConfig` |
| `risk_utils.rs` | Fee calculation |

## Tile Shape (Canonical Full Tile)

Per `specs/2/45-tiles.md` §3.2, risk is the **canonical**
tile shape: one pinned thread plus a tokio sidecar for
blocking Postgres I/O, with seven SPSC rings (rtrb) as the
only intra-process IPC.

### Rings (all in `main.rs::run_main`)

| Ring | Capacity | Direction | Site |
|------|----------|-----------|------|
| `PersistEvent` | 8192 | shard → persist sidecar | `main.rs:239` |
| `FillEvent` | 4096 | casting pump → shard | `main.rs:405` |
| `OrderRequest` (primary) | 2048 | casting pump → shard | `main.rs:407` |
| `MarkPriceUpdate` | 256 | casting pump → shard | `main.rs:409` |
| `BboUpdate` | 256 | casting pump → shard | `main.rs:411` |
| `OrderResponse` | 2048 | shard → casting sender | `main.rs:413` |
| `OrderRequest` (replica) | 2048 | casting pump → shard | `main.rs:415` |

`PersistEvent` is sized largest because Postgres write-behind
absorbs bursts of fills and position updates. The two
`OrderRequest` rings (primary + replica) exist so a failed
replica handoff cannot corrupt the live order stream.

### Threading

- **Pinned core** (`RSX_RISK_CORE_ID`, `main.rs:291-303`):
  runs `RiskShard` plus the casting receive pump. Busy-spin
  reactor, no blocking allowed.
- **Persist sidecar** (`std::thread::spawn`, `main.rs:260`):
  separate `tokio::runtime::Builder::new_current_thread`
  runtime that drains `PersistEvent` via
  `tokio_postgres`. Blocking PG write-behind cannot live
  on the pinned core, hence the dedicated thread.
- **Lease task**: `pg_try_advisory_lock` polling on the
  same sidecar pattern (replica path).

## Main Loop Priority

`run_main()` busy-spins on the pinned core in priority
order:

```
loop {
    1. Fills from all MEs        (highest — RECORD_FILL via casting)
    2. Order requests            (gateway → primary OrderRequest ring)
    3. Mark price updates        (RECORD_MARK_PRICE, main.rs:685)
    4. BBO updates               (trigger margin recalc)
    5. Funding settlement        (every 8h interval)
    6. Liquidation processing    (if triggered by fill)
    7. Lease renewal             (every ~1s)
    8. Tip-sync emit to replica  (record_type=0x20, main.rs:844)
}
```

## Pre-Trade Risk Check

`PortfolioMargin::check_order` (`margin.rs:98`) is the gate:

1. **Bypass for liquidation orders**: `order.is_liquidation
   → Ok(0)` (`margin.rs:107`).
2. **Bypass for reduce-only**: `order.reduce_only → Ok(0)`
   (`margin.rs:110`). Reduce-only orders clamp downstream
   in the matching path against the user's current position.
3. **Normal path**: recalc portfolio margin with current
   mark prices, compute `order_im = notional *
   initial_margin_rate`, reject if
   `available < order_im + worst_case_taker_fee`.
4. **Freeze**: `account.frozen_margin += order_im`.
5. **Route**: forward to ME via casting/UDP.

On `ORDER_DONE` the frozen amount is released. Frozen state
is in-memory only (not persisted; lost on restart, rebuilt
from open-order replay).

## Position Tracking

`FxHashMap<(user_id, symbol_id), Position>` in memory:

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

Position flip (long → short): close old at fill price
(realize PnL), open new with entry = fill price. Two-step
in `apply_fill`.

## Margin Calculation

```
equity         = collateral + sum(unrealized_pnl)
initial_margin = sum(|net_qty| * mark_price * im_rate)
maint_margin   = sum(|net_qty| * mark_price * mm_rate)
available      = equity - initial_margin - frozen_margin
```

Recalculated on every price tick for users with exposure in
the updated symbol. Exposure index (`Vec<Vec<u32>>`) maps
symbol → users.

## Funding

Every 8 hours (UTC 00:00 / 08:00 / 16:00):

```
premium         = (mark - index) / index
funding_payment = position_qty * mark * rate
```

Long pays short when rate > 0. **Invariant #9**: sum of all
funding payments across all users per symbol per interval
is zero. Idempotency key: `interval_id = epoch_secs / 28800`.

## Liquidation

When `equity < maint_margin`:

1. Enqueue user (`liquidation.enqueue`).
2. Each round: close largest position with `reduce_only +
   is_liquidation` market order (bypasses pre-trade margin
   per `margin.rs:107-112`).
3. Escalation: increase slippage tolerance per round
   (`base_slip_bps` → `max_slip_bps`, `max_rounds`).
4. Insurance fund absorbs losses past bankruptcy price.
5. Socialized loss if insurance exhausted.

## Replication & Failover

casting record type `0x20` carries `TipSyncMessage` between
main and replica (`main.rs:830-844` emit; `main.rs:1032`
ingest). Live path is casting/UDP; cold replay is replication/TCP from
WAL tip+1.

- **Main** (`run_main`, `main.rs:181`): acquire advisory
  lock → load from PG → replication from tip+1 → `CaughtUp`
  on all streams → live. Emits tip-sync to replica.
- **Replica** (`run_replica`, `main.rs:880`): `pg_try_advisory_lock`
  fails → buffer fills from all MEs → poll lock every
  ~500ms → on acquire, apply buffered fills up to last
  observed tip → promote.

### Advisory Lock (Invariant #10)

`AdvisoryLease` (`lease.rs`) wraps `pg_try_advisory_lock`
and `pg_advisory_unlock` on `shard_id`. Postgres guarantees
at most one main per shard.

### Promotion Path (T3.2 — known sharp edge)

Replica → main promotion at `main.rs:1086-1092` currently:

```rust
std::env::set_var("RSX_RISK_IS_REPLICA", "false");
run_main(shard_id, max_symbols)  // recursive
```

Two known issues, flagged in `.ship/13-A16Z-FIXES` T3.2:

1. `std::env::set_var` is UB-adjacent on glibc with
   concurrent reads (rust-lang/rust#27970).
2. The recursive `run_main` call grows the stack on every
   promotion.

Deferred until replication E2E coverage grows. Refactor
target: state-machine loop, no recursion, no env-var
toggle.

Data loss bound: 10ms single crash, 100ms dual crash.

## Persistence

Write-behind via the `PersistEvent` ring → persist sidecar
thread → `tokio_postgres`:

- Positions: batched UPSERT (advisory lock = single writer)
- Fills: COPY binary (bulk insert)
- Tips: batched UPSERT per `(instance_id, symbol_id)`
- `synchronous_commit = on`
- Backpressure: PG lag > 100ms stalls the hot path

## Deduplication

- Fills: `seq <= tips[symbol_id]` → skip (idempotent replay)
- Tips: monotonic, never decrease (**Invariant #5**)
- PG positions: version CAS on UPSERT (defensive)

## Performance Targets

| Path | Target |
|------|--------|
| Fill processing | <1us |
| Pre-trade check | <5us |
| Per-tick margin recalc | <10us/user |
| BBO → index price | <100ns |
| Postgres flush | every 10ms |
| Failover detection | ~500ms |

## Architectural Decisions

**Runtime: canonical full tile + tokio persist sidecar.** Risk
is the reference example of the full tile arrangement (per
[`../specs/2/45-tiles.md`](../specs/2/45-tiles.md) §3.2). The
hot thread is pinned, busy-spinning, draining seven SPSC
rings: fills, orders, mark prices, BBOs (consumers); order
responses, accepteds (producers); plus one `PersistEvent` ring
to the sidecar.

The persist sidecar is a separate OS thread running a
single-threaded tokio runtime so blocking `tokio_postgres`
writes — accounts, positions, fills, tips — cannot stall the
pinned core. The ring boundary is the chokepoint: full ring
stalls the hot path per the WAL backpressure rule (see
[`../notes/tiles.md`](../notes/tiles.md)).
