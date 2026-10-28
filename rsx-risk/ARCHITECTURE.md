# rsx-risk Architecture

Risk is the exchange's **pre-trade gate**. Before any order
reaches the matching engine, this process checks that the trader
can actually back it — that they hold enough collateral (margin)
to cover the new position if the market moves against them. An
order that fails the check is rejected here and never touches the
book.

That gate is what keeps one over-leveraged trader from running up
losses the exchange can't collect — the difference between a bad
trade and an insolvent exchange. On a perpetuals venue, every
position is leveraged and marked to a live price; without a margin
check in front of the book, a single account could take on risk
far past its collateral and leave the exchange holding the loss.
Risk is the process that says no.

It scales by user: one risk instance per user shard, each shard owning a range of
users and holding their positions and margin in RAM. Beyond the
gate, the same process tracks positions, settles funding,
liquidates underwater accounts, and runs the insurance fund, with
single-writer leader election via an eager warm-standby protocol
gated by a Postgres advisory lock (warm-catchup → caught-up →
non-blocking acquire). Canonical full-tile arrangement per
`specs/2/45-tiles.md` §3.2.

Specs: `specs/2/28-risk.md`, `specs/2/13-liquidator.md`,
`specs/2/45-tiles.md` §3.2.

## Trust Boundary

Risk is the **margin and solvency authority** — the trusted
enforcement point for whether an order is allowed to trade. The
concern is split by design: the **gateway** owns authentication
and structural validation (JWT, TLS, well-formedness — is this a
real, syntactically valid order from a real client?), and **risk**
owns the economic question (does this user have the margin and
position headroom for it?). The matching engine downstream trusts
risk's decision and does not re-check margin.

So "the matching engine doesn't validate input" is not a hole: the
validation happened here first. Risk is where an order is proven
solvent before it can rest in the book. This single-owner split is
the repo trust-boundary policy (`CLAUDE.md`;
`specs/2/47-validation-edge-cases.md`); the pre-trade check
(`PortfolioMargin::check_order`, below) is its implementation.

## Module Layout

Binary side (`main.rs` declares them):

| File | Purpose |
|------|---------|
| `main.rs` | Panic handler, tracing, `app::run` |
| `app.rs` | Composition root: config, health server, restart loop, `run_node` (PG load → warm catch-up → promotion → live loop → teardown) |
| `cast_loop.rs` | `CastIo` (the sockets) and `run`, the pinned busy-spin live loop |
| `metrics.rs` | `HealthSnapshot` from the load gauges |

Library side (`lib.rs`):

| File | Purpose |
|------|---------|
| `shard.rs` | `RiskShard` state, construction, `set_state`, `tick` dispatcher |
| `account.rs` | `Account`, fee deduction, `ensure_account` |
| `position.rs` | `Position`, `apply_fill`; `process_fill` and the exposure index upkeep |
| `margin.rs` | `PortfolioMargin`, `check_order`, `calculate_margin_needed`; `process_order` and the freeze lifecycle (`confirm_freeze`, `release_frozen_for_order`, `replay_freeze_order`) |
| `price.rs` | `IndexPrice`; BBO stash/drain, mark updates, the mark-with-index fallback |
| `funding.rs` | Rate, zero-sum settlement, `maybe_settle_funding` |
| `liquidation.rs` | `LiquidationEngine`; `check_liquidation_for`, halt/resume per symbol |
| `insurance.rs` | `InsuranceFund`, `process_socialized_loss` |
| `config.rs` | `ShardConfig` and `RuntimeConfig` from env; `SymbolRiskConfig` from Postgres; `apply_symbol_config`, `process_config_applied` |
| `persist.rs` | `PersistEvent`, the write-behind worker, `spawn_persist_worker`/`stop_persist_worker`, `push_persist` |
| `replay.rs` | `load_from_postgres`, `apply_record`, `drain_replay`/`handle_replay` for gap recovery |
| `promotion.rs` | `NodeState`, `run_warm_catchup` |
| `lease.rs` | `AdvisoryLease`, the lease-renewal thread |
| `schema.rs` | Migrations applied at boot |
| `db.rs` | Postgres connect helper |
| `rings.rs` | `ShardRings`, `OrderResponse`, `forward_to_gw` |
| `types.rs` | `FillEvent`, `BboUpdate`, `RejectReason` |
| `risk_utils.rs` | Fee calculation, retry jitter |

`RiskShard` is one struct; each domain module carries the
`impl RiskShard` block for the methods that mutate its state.

## Tile Shape (Canonical Full Tile)

Per `specs/2/45-tiles.md` §3.2, risk is the **canonical**
tile shape: one pinned thread plus a tokio sidecar for
blocking Postgres I/O, with SPSC rings (rtrb) as the only
intra-process IPC.

### Rings

| Ring | Capacity | Direction |
|------|----------|-----------|
| `PersistEvent` | 8192 | shard → persist sidecar |
| `SymbolRiskConfig` | 64 | persist sidecar → shard |
| `OrderResponse` | 2048 | shard → casting sender (to gateway) |
| `OrderRequest` (accepted) | 2048 | shard → casting sender (to ME) |

`PersistEvent` is sized largest because Postgres write-behind
absorbs bursts of fills and position updates. There are no input
rings: the casting receive pump and the shard share one pinned
thread, so the pump calls `shard.process_*` directly. Only shard
output, and the sidecar's config answers, cross a ring boundary.

### Threading

- **Pinned core** (`RSX_RISK_CORE_ID`, `app::run_node`): runs
  `cast_loop::run` — `RiskShard` plus the casting receive pump.
  Busy-spin reactor, no blocking allowed.
- **Persist sidecar** (`persist::spawn_persist_worker`): separate
  OS thread with a `tokio::runtime::Builder::new_current_thread`
  runtime that drains `PersistEvent` via `tokio_postgres`. Blocking
  PG write-behind cannot live on the pinned core, hence the
  dedicated thread. It also serves `ConfigReload` requests.
- **Lease thread** (`lease::spawn_lease_thread`): renews the
  advisory lock on its own tokio runtime; on loss it flips an
  `AtomicBool` the hot loop watches, and `run_node` tears down and
  re-enters warm catch-up.

## Live Loop

`cast_loop::run` busy-spins on the pinned core; each pass:

```
loop {
    1. ME streams (one record per stream): fills, BBO, order lifecycle,
       CONFIG_APPLIED — applied to the shard, forwarded to the gateway
    2. Gateway orders and cancels, until empty or an egress ring is full
    3. Mark prices, until empty
    4. shard.tick: config ring, liquidation sweep, BBO margin scans,
       funding settlement
    5. Drain the response ring (risk rejects → gateway) and the
       accepted ring (orders → their ME, re-sequenced per ME stream)
    6. Cast housekeeping, persist-worker liveness, lease health
}
```

A sticky FAULTED/RECONNECT on an ME stream drains the gap from that
ME's replication endpoint through `apply_record` and forwards the
records exactly like the live path; gateway and mark gaps are
skipped when no replication endpoint is configured.

## Pre-Trade Risk Check

`PortfolioMargin::check_order` is the gate:

1. **Bypass for liquidation orders**: `order.is_liquidation → Ok(0)`,
   so the liquidator can close dust left by partial fills.
2. **Unconfigured symbol**: no active `symbol_config_schedule` row
   → `SymbolNotConfigured`.
3. **Minimum order size**: the limit-price notional `price * qty`,
   in i128, must reach the symbol's `min_notional`, else
   `OrderTooSmall` (wire `ORDER_TOO_SMALL`, 14). This is the DoS
   answer to resting orders: each one freezes at least
   `min_notional * initial_margin_rate`. A reduce-only order skips it
   when it is IOC or FOK (it never rests) or when its qty equals the
   whole outstanding position on the symbol, at any TIF. A resting
   reduce-only order that closes less must meet it. Resting orders
   already below a raised minimum stay; only new orders are checked.
4. **Bypass for reduce-only**: `order.reduce_only → Ok(0)`.
   Reduce-only orders clamp downstream in the matching path against
   the user's current position.
5. **Leverage cap**: gross notional after the order must stay
   within `equity * max_leverage`. Then the normal path: recalc
   portfolio margin with current mark prices,
   `margin_needed = notional * initial_margin_rate / 10_000
   + taker_fee` (`calculate_margin_needed`), reject if
   `available < margin_needed`. A notional that overflows i64
   saturates to `i64::MAX` and fails here as `InsufficientMargin`.
6. **Freeze**: the amount is held in `frozen_orders` in memory;
   `available = equity - initial_margin - frozen`.
7. **Route**: forward to ME via casting/UDP.

The durable freeze is written only when the ME confirms the order
(`confirm_freeze` on `ORDER_ACCEPTED`), so Postgres can never hold a
freeze the WAL never confirmed. `ORDER_DONE` and `ORDER_FAILED` release it
in memory and in Postgres; `ORDER_CANCELLED` is a book event and neither
releases nor reaches the gateway. On restart,
freezes are rebuilt from the replication stream's `ORDER_ACCEPTED`
records (`replay_freeze_order`, the same arithmetic).

## Symbol Config

`config::load_symbol_configs` reads each symbol's active row —
highest `config_version` with `effective_at_ms <= now` — from
`symbol_config_schedule` at boot and again at promotion;
`RiskShard::apply_symbol_config` installs margin rates, leverage
cap, minimum order notional, fees and the version. A row whose
`min_notional` is NULL or not positive, or whose `price_decimals +
qty_decimals` is not `NOTIONAL_DECIMALS` (10), fails its load naming the
symbol, version and column: a boot load fails the boot, a live reload
leaves the symbol on its previous version. Margin sums `price * qty`
across symbols and adds it to collateral with no per-symbol scaling, so
every symbol shares the notional unit `10^-10` quote. Live changes follow the matching engine: a `CONFIG_APPLIED` record newer than the applied version
makes `process_config_applied` push `PersistEvent::ConfigReload`;
the persist sidecar fetches that exact version and returns it over
the config ring, which `tick` drains first. The pinned loop never
reads Postgres.

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
    version: u64,
}
```

Position flip (long → short): close old at fill price
(realize PnL), open new with entry = fill price. Two-step
in `apply_fill`. `positions_by_user` and `frozen_by_user` are
secondary indexes maintained at the same mutation points as their
primaries and rebuilt on `set_state`.

## Margin Calculation

```
equity         = collateral + sum(unrealized_pnl)
initial_margin = sum(|net_qty| * mark_price * im_rate)
maint_margin   = sum(|net_qty| * mark_price * mm_rate)
available      = equity - initial_margin - frozen_margin
```

Recalculated on every price tick for users with exposure in
the updated symbol. Exposure index (`Vec<Vec<u32>>`) maps
symbol → users. Mark price falls back to the BBO-derived index
when the mark feed is silent (`rebuild_fallback`).

## Funding

Every 8 hours (UTC 00:00 / 08:00 / 16:00):

```
premium         = (mark - index) / index
funding_payment = position_qty * mark * rate
```

Long pays short when rate > 0. **Invariant #9**: sum of all
funding payments across all users per symbol per interval
is zero; `funding::settle_symbol` parks the intra-shard rounding
residual on the largest position. Idempotency key:
`interval_id = epoch_secs / 28800`.

Exactly once per (symbol, interval): the tick settles every interval after
the symbol's last settled one, oldest first, at the latest mark and index.
Each settlement is one `PersistEvent::FundingSettlement` carrying the
payments and the changed account snapshots; `flush_batch` writes them with a
`funding_intervals` row in the batch's transaction, and a repeated interval
violates that table's primary key and rolls the batch back. Boot loads the
settled intervals in the same repeatable-read snapshot as the accounts. A
warm replica calls `follow_funding` after winning the advisory lock: it
charges the payments the previous main committed since the replica loaded
and advances its settled intervals, so promotion neither re-settles nor
drops them.

## Liquidation

When `equity < maint_margin`:

1. Enqueue each `(user, symbol)` position (`liquidation.enqueue`).
2. Each round: close the position with a `reduce_only +
   is_liquidation` IOC order at mark ± `round² * base_slip_bps`
   (bypasses the pre-trade margin check).
3. Escalation: linear delay, quadratic slippage, capped at
   `max_slip_bps`, up to `max_rounds`.
4. Past `max_rounds` the remaining position is socialized: the
   insurance fund absorbs the notional (`status = 3` row in
   `liquidations`) and the position re-enters the cascade after
   `socialized_cooldown_ns`.
5. An `ORDER_FAILED` for a liquidating user halts that user's
   symbols; the next fill on a symbol resumes it.

## Leader Election & Failover

Every risk-shard process is an identical **warm candidate
main**. Boot, standby, and promotion are one path: a process
loads Postgres, then *warms* — it applies the live main's
authoritative stream into its own shard state — and only goes
LIVE once it is caught up AND wins the advisory lock. There is
no separate "cold main boot"; promotion is always warm.

`app::run` calls `run_node` in a loop. `run_node` walks
`promotion::NodeState`:

```
enum NodeState { WarmCatchup, Live }

run_node:
  connect Postgres (NO advisory lock yet)
  → run_migrations
  → load_from_postgres           (accounts, positions, tips, ...)
  → apply_symbol_configs         (active symbol_config_schedule rows)
  ── NodeState::WarmCatchup ──────────────────────────────
  → consume ME WAL replication stream + mark stream,
    apply each record via replay::apply_record (NO persist,
    NO gateway ingress, NO egress, NO liquidation tick)
  → on caught_up: pg_try_advisory_lock (NON-BLOCKING)
       false → stay warm, keep applying, retry next poll
       true  → final-drain ME stream, transition to LIVE
  ── NodeState::Live ─────────────────────────────────────
  → apply_symbol_configs again (versions applied while warm)
  → spawn lease-renewal thread, persist sidecar
  → bind gateway receiver + senders (cast_loop::bind)
  → cast_loop::run until shutdown, lease loss or sidecar death
  → stop_persist_worker, stop_lease_thread
```

**What the warm standby consumes.** The SAME source the live
main uses for FAULTED recovery: each ME's WAL replication server in
`RSX_ME_REPLICATION_ADDRS`. Risk writes no WAL of its own.
Records are applied through `replay::apply_record` — so warm-apply
and gap-replay share one state-apply path. The mark cast stream
(`RSX_RISK_MARK_CAST_ADDR`) is drained into `update_mark` in both
warm and live modes.

**ME topology.** Risk owns one `CastReceiver` and one replay endpoint per
symbol. The live loop polls receivers round-robin, so sequence, gap, and reset
state never cross symbol boundaries. Warm standbys likewise require every
configured stream to reach its own caught-up fence before attempting promotion.
One-symbol deployments may use the singular address variables.

**Caught-up detection.** `rsx-cast`'s `ReplicationService`
emits `RECORD_CAUGHT_UP { live_seq }` after draining its
current WAL. `promotion::stream_to_caught_up` sets
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
the live loop starts with no gap: each stream is re-streamed to
`CAUGHT_UP` until `applied_seq` reaches the marker, at most 50
rounds, and a stream that ends without a marker fails the cycle.
The node then transitions to
LIVE with the already-warm shard — no discard, no full rebuild.

**Re-entry on lease loss.** The lease thread renews the lock
on its own tokio runtime; on loss it sets an `AtomicBool` the
live loop polls each tick. The loop returns `LiveExit::LeaseLost`;
`run_node` tears down the persist sidecar and lease thread (each
owns a PG connection) and returns `Ok(())`. `app::run` calls
`run_node` again, which re-enters WARM CATCHUP and re-tries the
non-blocking lock — this process becomes a warm standby again.
Each `run_node` cycle runs on its own thread, so the hot pin ends
with the cycle and the next cycle's side-threads start unpinned.
The crash backoff sleeps in 100 ms slices and ends on SIGTERM.
`run_node` owns its PG client, catch-up consumers, persist worker,
lease thread, and sockets, and tears them all down before
returning, so a demote → re-acquire cycle leaks nothing. A dead
persist sidecar (circuit open after sustained flush failure) is
treated the same way, as `Err`, so the restart budget applies. On
SIGTERM/SIGINT the loop returns `LiveExit::Shutdown`, the sidecars
are stopped (the persist worker flushes what it holds) and the
process exits without re-entering catch-up.

### Advisory Lock (Invariant #10)

`AdvisoryLease` (`lease.rs`) wraps `pg_try_advisory_lock`
(promotion gate, non-blocking), the `pg_locks` self-check
(`renew`), and `pg_advisory_unlock` (`release`) on `shard_id`.
Postgres guarantees at most one holder per shard, so at most
one main per shard. Catch-up never bypasses this — it only
decides *when* to call `try_acquire`.

Data loss bound: 10ms (one WAL flush interval) on a single
crash, recovered by replaying the ME stream from the persisted tip.
Async replication adds a bounded staleness window on promotion (the
main can apply record K and die before the replication server
streams K to standbys); see CRASH-SCENARIOS.md.

## Persistence

Write-behind via the `PersistEvent` ring → persist sidecar
thread → `tokio_postgres`, one transaction per 10 ms flush:

- Positions, accounts, insurance fund, tips: row-at-a-time UPSERT
  (advisory lock = single writer; no version guard)
- Frozen orders: UPSERT on `ORDER_ACCEPTED`, DELETE on release
- Fills, funding, liquidations: row INSERTs (append-only)
- Ring full → the hot path stalls in `push_persist`, never drops;
  eight consecutive failed flushes open the circuit, the worker
  exits, and the live loop demotes

## Deduplication

- Fills: `seq <= tips[symbol_id]` → skip (idempotent replay)
- Tips: monotonic in memory, never decrease (**Invariant #5**)
- Freezes: keyed by the 128-bit order id; insert and release are
  idempotent

## Performance Targets

| Path | Target |
|------|--------|
| Fill processing | <1us |
| Pre-trade check | <5us |
| Per-tick margin recalc | <10us/user |
| BBO → index price | <100ns |
| Postgres flush | every 10ms |
| Failover detection | ~500ms |

## Performance evidence

Risk benchmarks live under `benches/`, with dated runs in
`../reports/`. Exact latency tables are omitted here until the same workload
records p50 and p99. The open-loop capacity harness reports accepted rate,
queueing, and tail latency rather than turning a closed-loop median into a
capacity claim.

## Architectural Decisions

**Runtime: canonical full tile + tokio persist sidecar.** Risk
is the reference example of the full tile arrangement (per
[`../specs/2/45-tiles.md`](../specs/2/45-tiles.md) §3.2). The
hot thread is pinned, busy-spinning, draining the cast sockets
directly and three SPSC rings: order responses and accepteds
(producers) plus the `PersistEvent` ring to the sidecar, with the
config ring coming back.

The persist sidecar is a separate OS thread running a
single-threaded tokio runtime so blocking `tokio_postgres`
writes — accounts, positions, fills, tips — cannot stall the
pinned core. The ring boundary is the chokepoint: full ring
stalls the hot path per the WAL backpressure rule (see
[`../docs/concepts/02-tiles-and-pinning.md`](../docs/concepts/02-tiles-and-pinning.md)).

## Lock Order

None. The hot-path tile is single-threaded (one pinned thread
owns `RiskShard`); cross-thread state handoff is exclusively
through SPSC rings. The persist sidecar uses its own Postgres
client — no shared locks between tiles. Only postgres-side
row/advisory locks exist (see `lease.rs`: `AdvisoryLease`),
held solely by the lease thread's tokio runtime, never by the
pinned tile. Adding a `Mutex`/`RwLock`/`DashMap` requires
documenting the acquisition order here.
