# rsx-risk Architecture

Risk engine process. One instance per user shard. Pre-trade
margin checks, position tracking, funding, liquidation,
insurance fund, and single-writer leader election via an
eager warm-standby protocol gated by a Postgres advisory
lock (warm-catchup → caught-up → non-blocking acquire).
Canonical full-tile arrangement per
`specs/2/45-tiles.md` §3.2.

Specs: `specs/2/28-risk.md`, `specs/2/45-tiles.md` §3.2.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: warm-catchup + promotion state machine, ring construction, casting pump, lease watch |
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
| `rings.rs` | `ShardRings`, `OrderResponse`, `MarkPriceUpdate` |
| `config.rs` | `ShardConfig`, `ReplicationConfig` |
| `risk_utils.rs` | Fee calculation |

## Tile Shape (Canonical Full Tile)

Per `specs/2/45-tiles.md` §3.2, risk is the **canonical**
tile shape: one pinned thread plus a tokio sidecar for
blocking Postgres I/O, with seven SPSC rings (rtrb) as the
only intra-process IPC.

### Rings (all in `main.rs::run_main`)

| Ring | Capacity | Direction |
|------|----------|-----------|
| `PersistEvent` | 8192 | shard → persist sidecar |
| `OrderResponse` | 2048 | shard → casting sender |
| `OrderRequest` (accepted) | 2048 | shard → casting sender (to ME) |

`PersistEvent` is sized largest because Postgres write-behind
absorbs bursts of fills and position updates. Input rings were
removed: the casting receive pump and the shard share one
pinned thread, so the pump calls `shard.process_*` directly
(an input ring would be a redundant per-message copy). Only
shard output crosses a ring boundary.

### Threading

- **Pinned core** (`RSX_RISK_CORE_ID`, `main.rs:291-303`):
  runs `RiskShard` plus the casting receive pump. Busy-spin
  reactor, no blocking allowed.
- **Persist sidecar** (`std::thread::spawn`, `main.rs:260`):
  separate `tokio::runtime::Builder::new_current_thread`
  runtime that drains `PersistEvent` via
  `tokio_postgres`. Blocking PG write-behind cannot live
  on the pinned core, hence the dedicated thread.
- **Lease thread**: renews the advisory lock on its own
  tokio runtime; on loss it flips an `AtomicBool` the hot
  loop watches, triggering a clean `run_main` re-entry.

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
    7. Lease health check        (AtomicBool set by lease thread)
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

## Leader Election & Failover

Every risk-shard process is an identical **warm candidate
main**. Boot, standby, and promotion are one path: a process
loads Postgres, then *warms* — it applies the live main's
authoritative stream into its own shard state — and only goes
LIVE once it is caught up AND wins the advisory lock. There is
no separate "cold main boot"; promotion is always warm.

`main()` calls `run_main` in a loop. `run_main` is a two-state
machine (`NodeState`):

```
enum NodeState { WarmCatchup, Live }

run_main:
  connect Postgres (NO advisory lock yet)
  → run_migrations
  → load_from_postgres           (accounts, positions, tips)
  → replay_from_wal(tip+1)       (fold boot WAL into shard)
  ── NodeState::WarmCatchup ──────────────────────────────
  → consume ME WAL replication stream + mark stream,
    apply each record via replay::apply_record (NO persist,
    NO gateway ingress, NO egress, NO liquidation tick)
  → on caught_up: pg_try_advisory_lock (NON-BLOCKING)
       false → stay warm, keep applying, retry next poll
       true  → final-drain ME stream, transition to LIVE
  ── NodeState::Live ─────────────────────────────────────
  → attach persist producer + spawn persist sidecar
  → spawn lease-renewal thread
  → bind gateway receiver + senders (ingress + egress)
  → live loop: apply ME records AND forward to GW,
    process gateway orders, run liquidation tick
```

**What the warm replica consumes.** The SAME source the live
main uses for FAULTED recovery: ME's WAL replication server at
`RSX_ME_REPLICATION_ADDR`. No separate risk WAL is introduced.
Records are applied through `replay::apply_record` — the exact
function `replay_from_wal` uses — so warm-apply, boot-replay,
and FAULTED-replay share one state-apply path. The mark cast
stream (`RSX_RISK_MARK_CAST_ADDR`) is drained into
`update_mark` in both warm and live modes.

**ME topology.** The live main binds ONE `CastReceiver` for
all MEs (single recv addr) and replays a single `stream_id`
(the first/primary ME's `symbol_id`) for FAULTED recovery. The
warm replica matches that topology exactly: ONE
`ReplicationConsumer` against `RSX_ME_REPLICATION_ADDR` with
that same `stream_id`. (If the main is ever changed to
per-symbol ME receivers, the warm path must grow to one
consumer per ME stream to match.)

**Caught-up detection.** `rsx-cast`'s `ReplicationService`
emits `RECORD_CAUGHT_UP { live_seq }` after draining its
current WAL. The warm loop sets
`caught_up ⟺ saw RECORD_CAUGHT_UP(live_seq=T) AND
applied_seq >= T`. The consumer uses a per-node tip file, so a
disconnect/error clears caught-up implicitly (re-derived next
iteration) and reconnect resumes from the persisted tip+1.
`CAUGHT_UP` carries no seq so it never advances the tip.

**Promotion (strict, catch-up-only) and the no-double-main
argument.** `pg_try_advisory_lock` is called ONLY when
caught_up. The advisory lock — not catch-up — remains the
SOLE single-main fence (invariant #10); catch-up only gates
*when* `try_acquire` is called, it never replaces the lock.
So there is no double-main window: two nodes can both be
caught up, but Postgres grants the lock to exactly one. The
loser stays warm and retries `try_acquire` every
`lease_poll_interval_ms`. There is NO cold-promote fallback:
a node that never catches up never attempts the lock (strict
availability tradeoff — see CRASH-SCENARIOS.md). On winning
the lock the node does a FINAL DRAIN (apply any ME records
written between the last `CAUGHT_UP` and the lock grant) so
the live loop starts with no gap, then transitions to LIVE
with the already-warm shard — no discard, no full rebuild.

**Re-entry on lease loss.** The lease thread renews the lock
on its own tokio runtime; on loss it sets an `AtomicBool` the
live loop polls each tick. On loss the loop tears down the
persist sidecar and lease thread (each owns a PG connection)
and returns `MainTransition::Demote`. `main()` calls `run_main`
again, which re-enters WARM CATCHUP (step 2) and re-tries the
non-blocking lock — this process becomes a warm standby again.
`run_main` is re-enterable: it owns its PG client, catchup
consumer, persist worker, lease thread, and sockets, and tears
them all down before returning, so a Demote → re-acquire cycle
leaks nothing. On a crash (error return) `main()` applies the
restart-backoff budget instead.

### Advisory Lock (Invariant #10)

`AdvisoryLease` (`lease.rs`) wraps `pg_try_advisory_lock`
(promotion gate, non-blocking), the `pg_locks` self-check
(`renew`), and `pg_advisory_unlock` (`release`) on `shard_id`.
Postgres guarantees at most one holder per shard, so at most
one main per shard. Catch-up never bypasses this — it only
decides *when* to call `try_acquire`.

Data loss bound: 10ms (one WAL flush interval) on a single
crash, recovered by WAL replay from the persisted tip. Async
replication adds a bounded staleness window on promotion (the
main can apply record K and die before the replication server
streams K to standbys); see CRASH-SCENARIOS.md.

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

## Lock Order

None. The hot-path tile is single-threaded (one pinned thread
owns `RiskShard`); cross-thread state handoff is exclusively
through SPSC rings. The persist sidecar uses its own Postgres
client — no shared locks between tiles. Only postgres-side
row/advisory locks exist (see `lease.rs`: `AdvisoryLease`),
held solely by the main-thread tokio runtime, never by the
pinned tile. Adding a `Mutex`/`RwLock`/`DashMap` requires
documenting the acquisition order here.
