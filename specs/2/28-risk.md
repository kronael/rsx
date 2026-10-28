---
status: partial
---

# Risk Engine Specification

## Table of Contents

- [Context](#context)
- [Architecture Overview](#architecture-overview)
- [Return Path (output leg)](#return-path-output-leg)
- [Components](#components)
- [Persistence](#persistence)
- [Replication & Failover](#replication--failover)
- [Main Loop Pseudocode](#main-loop-pseudocode)
- [File Organization](#file-organization)
- [Performance Targets](#performance-targets)
- [Implementation Phases](#implementation-phases)
- [Tests](#tests)

---

## Context

The risk engine sits between gateway and matching engines. It performs
pre-trade margin checks, ingests orderbook events to update positions,
tracks mark/index prices for risk and funding, and persists state to a
durable store (Postgres in v1). Order intents at ingress are not WAL’d;
they may be lost on risk crash before execution. Risk must recover from
orderbook WAL replay.

## Architecture Overview

```
Mark Aggregator -+  (see MARK.md)
                  |
           [casting/UDP: MarkPriceRecord, via RSX_RISK_MARK_CAST_ADDR]
                  |
Gateway -[casting/UDP]-> Risk Shard (main) -[casting/UDP]-> Matching Engines
                       |        ^                     |
                       |        | [casting/UDP: BBO + fills]|
                       |        +---------------------+
                       |
                  [replica sync TBD]
                       |
                  Risk Shard (replica)
```

- **N risk shards** by `user_id` hash, each with a dedicated replica
- Each shard consumes **all symbols** (filters fills for its users)
- Hot path is single-threaded, busy-spin on dedicated core
- Postgres write-behind on separate thread (10ms flush)
- The output leg (fills → client) is moving off Risk to **ME → GW
  direct**, with `ME → Risk` async for settle — see
  [Return Path](#return-path-output-leg). The diagram's fills→Risk
  arrow is that async settle stream.

### Tile arrangement

One pinned thread runs the cast receive pump and the shard state
machine together (`rsx-risk/src/cast_loop.rs`): records are applied
by direct `process_*` calls, so there are no input rings. Four SPSC
rings (rtrb) cross the thread boundary.

| Ring               | Direction              | Capacity | Item type          |
|--------------------|------------------------|---------:|--------------------|
| `persist`          | shard → persist worker |     8192 | `PersistEvent`     |
| `config`           | persist worker → shard |       64 | `SymbolRiskConfig` |
| `response`         | shard → gateway egress |     2048 | `OrderResponse`    |
| `accepted`         | shard → ME egress      |     2048 | `OrderRequest`     |

Ring-full policy: persist-full → the producer stalls (never drops);
an egress ring full → the loop stops draining the gateway socket
and skips the liquidation tick until it drains, so the kernel
receive buffer absorbs the burst.

The persist sidecar runs on its own `tokio::runtime::current_thread`
in a dedicated `std::thread::spawn` (`persist::spawn_persist_worker`
— separate from the lease runtime). It owns its own Postgres client
and shares no locks with the pinned tile.

## Return Path (output leg)

Risk is the margin authority and **brackets** the matching engine: it
*reserves* margin before execution and *settles* it after. But only the
reserve is synchronous on the order's path — the settle is **async**.

```
                 reserve (sync)            execute
Gateway --[order]--> Risk --[order]--> Matching Engine
   ^                  ^                      |
   |                  | ME→Risk (async:      | ME→GW (direct:
   |                  |  fills, BBO — settle) |  fill / DONE / FAILED)
   |                  +----------------------+
   +-------------------------------------------+
```

- **Input leg — reserve (synchronous):** `GW → Risk → ME`. Pre-trade
  check + **freeze worst-case margin** (`notional × im_rate + fee`),
  then forward. A hard gate — without it ME could execute an order the
  user can't back. See [Pre-Trade Risk Check](#6-pre-trade-risk-check).
- **Confirmation — `ME → GW` direct (critical path):** the fill /
  `ORDER_DONE` / `ORDER_FAILED` reaches the client in **3 hops**. The
  gateway is a **direct casting consumer of ME** (alongside marketdata),
  tracking per-symbol seq for gap detection.
- **Settle — `ME → Risk` (async, off critical path):** Risk applies the
  fill to the position and releases the frozen reservation (`apply_fill`
  on FILL, `release_frozen_for_order` on DONE / CANCELLED / FAILED). Not
  on the client path.

Client-visible path is **3 hops** (vs 4 if Risk forwarded), ~7.6 µs
saved (~half the round-trip; see the casting loopback RTT bench).

Why it is safe:

1. The input freeze is **worst-case** → no over-leverage in the async
   gap; the order was already margin-gated on the way in.
2. Fills are **authoritative** — Risk reconciles *to* them, never
   rejects. The ME confirmation is always valid.
3. Risk recovers via **orderbook-WAL replay** (gap detection unchanged),
   so being off the return leg does not affect crash-safety.

Trade-off — **margin is eventually-consistent (no read-your-writes).**
The client learns order A's outcome from ME before Risk processed A's
release; a sub-millisecond client reusing freed margin within the async
lag (≈ one `ME→Risk` hop + Risk queue, µs-scale) can get order B
spuriously rejected / under-credited. **Solvency is never at risk** —
only a fast client racing its own freed margin. A capital/UX cost, not
a safety one; acceptable for the demo.

**Hybrid escape hatch** (if read-your-writes is later required, e.g. to
court capital-recycling HFT market-makers): keep `ME → GW` for fast
notifications, but have the gateway hold a per-user "next
margin-affecting order waits for Risk's margin-update ack" — restoring
read-your-writes at the cost of gateway state + a thin `Risk → GW`
margin-delta stream (not the full fill).

### Reply delivery & idempotency

The client gets **no reply until a response propagates back** — there is
no optimistic gateway ack. So a crash, a dropped datagram, or a WS
timeout anywhere on the path means the client **may never receive the
reply**, even though the order may have executed. This is inherent: the
order's outcome is durable in the **ME WAL** (a fill is journaled before
anyone is told); only the *notification* is at-most-once.

Recovery is **exactly-once execution + at-most-once notification +
idempotent retry**:

1. **`cid` idempotent resubmit** — the gateway binds `(user_id, cid)` to
   the order id it minted, in memory, for 1 hour, and answers a repeated
   `cid` itself without forwarding. The ME's dedup on `(user_id, order_id)`
   is persisted in the WAL (`RECORD_ORDER_ACCEPTED`) over the same hour. A
   resend after a gateway restart is a new order; a resend never re-submits
   an order that was lost before it reached risk.
2. **Reconcile on reconnect** — the client queries open-orders /
   fills-since-seq and learns the true state from the durable log.

**Order expiry (`valid_until`) — NOT YET SUPPORTED.** TimeInForce is only
`GTC | IOC | FOK`; there is no client-set good-till-date. A `valid_until`
timestamp would bound the worst case (a resting order whose owner lost
the connection self-cancels at its deadline, releasing its freeze) and
is the recommended addition. It does **not** by itself fix the
orphan-freeze below (an order ME never accepted has nothing to expire) —
that needs WAL reconciliation.

### Failure handling (every step)

| Step | Logical fail | Crash / loss |
|------|--------------|--------------|
| Client→GW | malformed/auth → GW rejects to client over WS | GW stateless → client reconnects, resubmits (`cid`) |
| GW→Risk | — | intents **not WAL'd** → lost → client timeout → resubmit |
| Risk check+freeze | insufficient margin → **`Risk → GW` reject** (the one client-bound msg that originates at Risk, not ME — this reverse edge cannot be removed) | freeze applied + write-behind to PG **pre-send** → orphan freeze if Risk dies before ME accepts (see below) |
| Risk→ME | — | order lost pre-exec → resubmit; freeze self-heals (never reconstructed from WAL) |
| ME match | post-only-cross and IOC remainder→`DONE` (cancelled); FOK/reduce-only→`FAILED` — all ME-originated → `ME→GW` + `ME→Risk` | WAL authoritative to last flush; hot-spare ME + WAL replay rebuilds book |
| ME→GW (confirm) | — | **confirmation lost ≠ fill lost**; client reconciles via fills-since-seq |
| ME→Risk (settle) | — | Risk behind/crashes → WAL replay rebuilds positions (`apply_fill`) + freezes (`replay_freeze_order` from `OrderAccepted`) + releases |

**Invariants that make every crash survivable:** `cid` idempotency;
ME WAL is the authority; freezes are rebuilt from WAL `OrderAccepted`
(so a freeze ME never accepted self-heals); worst-case freeze makes async
settle safe; tips monotonic ⇒ idempotent replay from tip+1.

### Open bug — orphan freeze (reconcile from WAL)

`process_order` (`shard.rs`) freezes in-memory **and write-behinds a
`FrozenInsert` to PG before forwarding to ME**. If Risk dies (or the
`Risk→ME` send drops) after that PG write but before ME accepts the
order, recovery loads the PG snapshot with a freeze that has **no
`OrderAccepted` and no release in the WAL** — a phantom hold on the
user's margin that never clears.

**Fix — the WAL `OrderAccepted` stream is the sole authority for
freezes.** On recovery, reconcile the PG `frozen_orders` snapshot against
the WAL: drop any frozen order the WAL never confirms. Equivalently,
write-behind the durable freeze **on `OrderAccepted` ingestion** (ME
confirmed) rather than pre-send, so PG can never hold an unconfirmed
freeze. The pre-send in-memory freeze (needed for the pre-trade gate)
stays; only its *durable* record moves to the WAL-confirmed point.

> **Status — `partial`.** The input leg + worst-case freeze are
> shipped. The async output split is **not yet implemented**: code today
> still routes the output leg back through Risk (`forward_to_gw` + the
> `response` shard→gateway ring). Reaching this spec means ME adds the
> gateway as a fan-out target, the gateway tracks per-symbol seq, and
> Risk drops `forward_to_gw` / the `response` ring. **rsx-cast is
> unchanged** (caller-level fan-out only — the freeze holds).

## Components

### 1. Orderbook Event Ingestion

- casting/UDP receiver from each matching engine
- Each fill contains `taker_user_id`, `maker_user_id`, `symbol_id`,
  `seq`
- Filter: `user_in_shard(user_id)` via range or bitmask check
- Dedup: skip if `seq <= tips[symbol_id]`
- Apply fill to both taker and maker positions (if in shard)
- Fee calculation on each fill:
  - `taker_fee = floor(qty * price * taker_fee_bps / 10_000)`
  - `maker_fee = floor(qty * price * maker_fee_bps / 10_000)`
    (Floor always — exchange keeps the sub-tick remainder.
    Integer division in Rust truncates toward zero, which
    equals floor for positive values.)
  - Deduct: `taker.collateral -= taker_fee`
  - Deduct: `maker.collateral -= maker_fee` (negative fee =
    rebate credited)
  - Fee rates from symbol config (METADATA.md)
  - Persist fee with fill record
- Apply order_done/cancel/failed to release frozen margin
- Advance per-symbol tip after processing
- Push to persistence rings (positions, fills, tips)
- Push to replica ring (tip sync)
- Apply config updates from matcher `CONFIG_APPLIED` events: a
  version newer than the one applied is fetched from
  `symbol_config_schedule` by the persist sidecar and installed on
  the next tick. On cold start and on promotion, Risk loads each
  symbol's active row (highest version effective now) from the same
  table. Postgres is the source of truth; `CONFIG_APPLIED` says
  when the matcher switched. See [19-metadata.md](19-metadata.md).
- Forward `CONFIG_APPLIED` to Gateway for cache sync

### 2. Position Manager

In-memory `FxHashMap<(user_id, symbol_id), Position>` and `FxHashMap<u32, Account>`.

See `rsx-risk/src/position.rs` for `Position` and `rsx-risk/src/account.rs` for `Account`.

### 3. Margin Calculator (Portfolio Margin)

Portfolio margin: considers all positions across all symbols for a
user.

**Recalculate on every tick.** Sharding keeps load manageable.

**Exposure index** -- Vec indexed by symbol (u8/u16):
```rust
// Symbols have a compact index (u8 or u16, limits max symbols)
// Vec of user_ids with open positions per symbol
exposure: Vec<Vec<u32>>,  // exposure[symbol_idx] = [user_ids...]
```

Updated on fill (add user) and position close (remove user).
On each price tick (BBO or mark price update), recalculate
margin for all users with exposure in that symbol. Scale
horizontally by adding more user shards.

**Formulas** (all values fixed-point integers):

Per-position: `net_qty = long_qty - short_qty`, `notional = |net_qty| * mark_price`,
`unrealized_pnl = net_qty * (mark_price - avg_entry)`.

Per-user: `equity = collateral + sum(unrealized_pnl)`,
`available = equity - initial_margin - frozen_margin`.

Pre-trade: accept if `available >= order_notional * im_rate + taker_fee`.
Liquidation trigger: `equity < maint_margin`.

Edge cases: empty position → upnl=0; position flip → realize PnL, open new
entry at fill price (two-step in `apply_fill`); mark unavailable → use index price.

See `rsx-risk/src/margin.rs` for `PortfolioMargin`, `SymbolRiskParams`, and
`MarginState`. Margin recalculated on every price tick for all exposed users.

### 4. Price Feeds

**From matching engines** (casting/UDP, same as fills):
- BBO updates: `(symbol_id, best_bid, best_bid_qty, best_ask,
  best_ask_qty)`
- BBO is also derived by MARKETDATA from its shadow orderbook
  (see [MARKETDATA.md](MARKETDATA.md) section 4)
- Risk engine calculates **index price** per symbol:
  `index = (best_bid * ask_qty + best_ask * bid_qty)
           / (bid_qty + ask_qty)`
- If `bid_qty + ask_qty == 0`: use last known index
- If only one side has qty: use that side's price
- If no BBO ever received: use mark price
- O(1) per BBO update, stored in `Vec<IndexPrice>`

**From Mark Price Aggregator** (casting/UDP, see [MARK.md](MARK.md)):
- Risk engine receives `MarkPriceRecord` over casting/UDP
  from the mark process.
- Risk engine stores mark prices in `Vec<i64>`.
- Used for margin/risk calculations (unrealized PnL, liquidation).
- Fallback: if mark price is missing (`mark==0`) and an index
  price is available (`index.valid==true`), risk uses index
  price as the mark for margin/liquidation checks.

### 5. Funding Engine

Logically part of risk engine, could be extracted later.

- **Funding rate** = f(mark_price, index_price)
  - `premium = (mark_price - index_price) / index_price`
  - Rate clamped to bounds, formula TBD per symbol config
- **Interval**: 8h (periodic), configurable per symbol
- **Application**: at interval boundary, iterate all positions for
  each symbol, apply funding payment/charge:
  - Long pays short when funding rate > 0 (mark > index)
  - Short pays long when funding rate < 0 (mark < index)
  - `funding_payment = position_qty * mark_price * funding_rate`
- Rate calculated continuously (updated on each price tick)
- Applied atomically at settlement time
- Settlement: UTC 00:00, 08:00, 16:00
- **Idempotency key:** `interval_id = unix_epoch_secs / 28800`
  (28800 = 8 hours). Each funding settlement is keyed by
  `(symbol_id, interval_id)`. Duplicate settlement for same
  interval_id is a no-op.
- **Clock requirement:** NTP required on all hosts. Maximum
  allowed clock skew: 100ms. Funding settlement uses wall
  clock; skew >100ms could cause interval_id mismatch between
  components.
- Missed intervals: settle on next startup
- Mark price at settlement = latest available
- Funding payments persisted to Postgres (append-only)
- **Persisted state:** the last settled `interval_id` per (shard,
  symbol) is the max of `funding_intervals`. One settlement of one
  symbol and interval is one persist event, so its payments, the
  collateral they changed and its `funding_intervals` row commit in
  one transaction; recording the same interval twice fails that
  transaction. A node loads the settled intervals with its accounts
  in one snapshot, and a warm replica applies the intervals the
  previous main settled while it was warm before its first tick.
- **Catch-up:** each interval after the persisted one, up to the
  current one, is settled in order. Every missed interval uses the
  latest mark and index (the rule above), so a catch-up charges the
  current premium once per missed interval. A symbol with no
  persisted interval settles the current one. A symbol with open
  positions and no mark or index price waits and retries each second.

### 6. Pre-Trade Risk Check

Order flow through `RiskShard::process_order`:

1. Reject `NotInShard` if this shard does not own the user's virtual
   shard: `vshard = hash(user_id) % N_VSHARDS` (N_VSHARDS fixed),
   reject when `shardmap[vshard] != shard_id`. See §Sharding & scale-out.
2. Recompute fallback mark (mark with index backfill) on first
   order after a price update
3. Reject `UserInLiquidation` if `equity < maintenance_margin`
   and the order is not itself a liquidation
4. Liquidation orders (`is_liquidation == true`) bypass the
   minimum-size and margin checks (`Ok(0)`)
5. Reject `SymbolNotConfigured` (wire: `SymbolNotFound`) if the
   symbol has no active `symbol_config_schedule` row
6. Reject `OrderTooSmall` (wire: `ORDER_TOO_SMALL`, 14) if the
   limit-price notional `price * qty` (i128, no overflow) is below the
   symbol's `min_notional`. The minimum prices a flood of resting
   orders: each one freezes at least `min_notional * im_rate`.
   Exempt: a reduce-only IOC or FOK order (it never rests) and a
   reduce-only order of any TIF whose qty equals the whole outstanding
   position (`|net_qty|`) of the user on that symbol. A resting
   reduce-only order that closes less must meet the minimum. Orders
   already resting below a raised minimum stay; only new orders are
   checked. `min_notional` is read from `symbol_config_schedule`,
   NOT NULL and `> 0`, in the unit of `price * qty`; a row without one
   fails the load
7. Reduce-only orders skip margin reservation (`Ok(0)`) — the
   eventual fill releases existing margin; no new margin needed.
   Position-side enforcement (reduce-only must actually reduce)
   lives in the matching engine, not here
8. Reject `InsufficientMargin` if gross notional after the order
   exceeds `equity * max_leverage`
9. Otherwise compute `margin_needed = order_notional * im_rate /
   10_000 + taker_fee`; reject `InsufficientMargin` if
   `available_margin < margin_needed`; else freeze and emit
   `OrderResponse::Accepted`

The shard does **not** enforce max-position caps or a kill-switch.
Those are not in scope for v1; if added, they belong upstream of
the margin check.

On fill: update position; margin recalculated on next price tick.
On ORDER_DONE: release frozen margin for that order.

See `rsx-risk/src/margin.rs::process_order`.

The freeze is held in memory from acceptance; its durable
`frozen_orders` row is written only when the ME confirms the order
(`ORDER_ACCEPTED`) and deleted on ORDER_DONE/CANCELLED/FAILED. On
restart, replication-stream replay reconstructs frozen_orders from
ORDER_ACCEPTED records (which contain price, qty, symbol_id) with
the same arithmetic the gate uses.

### 7. Per-Tick Margin Recalc

On each price update (BBO or mark price): iterate all users with exposure in the
updated symbol, recalculate full portfolio margin, enqueue liquidation if
`equity < maint_margin`.

**No liquidation race condition:** single-threaded main loop serializes fills and
liquidation processing — a fill updates the position before
`maybe_process_liquidations()` runs.

See `rsx-risk/src/price.rs::drain_stashed_bbos` and
`rsx-risk/src/liquidation.rs::check_liquidation_for`.

## Sharding & scale-out

Risk shards by **user**. The mapping is two stages, deliberately
decoupled so the cluster can grow without a global reshuffle:

```
user_id ──hash──▶ vshard = hash(user_id) % N_VSHARDS   (N_VSHARDS FIXED, e.g. 4096)
vshard  ──map───▶ shard_id                              (shardmap, mutable lookup table)
```

`N_VSHARDS` is a constant chosen once and never changed, so `user → vshard`
is stable forever. Physical placement lives entirely in `shardmap`
(vshard → shard_id), a small table the gateway and every shard read.

**Why not `user_id % shard_count`.** Plain modulo puts the live node
count in the hash, so adding a shard (4→5) reassigns ≈ (1 − 1/N) of *all*
users at once. Because a risk shard holds the user's positions + frozen
margin in RAM and is their solvency authority, that means migrating
nearly everyone's live financial state in one step — there is no
incremental path. Fixed vshards + a map confine a node addition to moving
≈ 1/nodes of the *slots*; every other user stays put.

**Adding a shard.** Pick a set of vshards to hand to the new node, then
migrate each (below) and flip its `shardmap` entry. Nothing else moves.

**User migration (vshard A → B).** Reuses the warm-standby machinery in
§Replication & Failover — a migration is a scoped failover:

1. **Warm-load.** B loads the vshard's users' positions from Postgres and
   catches up to the WAL tip via replication, *while A still serves them*.
2. **Cutover.** Briefly quiesce the vshard on A (stop accepting new orders
   for those users, drain in-flight, flush final state to WAL/PG), set
   `shardmap[vshard] = B`, and B replays the last records to the tip and
   goes live.
3. **Release.** A drops the vshard's users.

The only pause is per-vshard (a slice of users), never global. The
per-shard advisory lock (invariant #10, one main per shard) generalizes
to **per-vshard**: exactly one node is the live solvency authority for a
vshard at any instant, so the cutover cannot split-brain on margin.

**Status:** forward design. v1 runs single-shard (`shard_count = 1`,
`N_VSHARDS` maps everything to shard 0); the gateway routes via a single
`RSX_RISK_CAST_ADDR`. The `shardmap` lookup, multi-shard routing, and
migration handshake are specced here but not yet implemented.

## Persistence

**Retention (v1):** Postgres keeps per-user order state and positions. History
retention in Postgres is a v1 choice; v2 will move long-term history off
Postgres.

### Postgres Schema

Tables: `positions`, `accounts`, `fills`, `tips`, `frozen_orders`,
`insurance_fund`, `liquidations`, `funding`, plus the symbol catalog
(`symbol_static`, `symbol_config_schedule`, `symbol_config_applied`).

See `rsx-risk/migrations/` for DDL.

### Write Patterns

All persistence happens on a **separate write-behind thread**:
- Hot path uses in-process SPSC rings only for internal queues
- Write-behind thread drains rings every 10ms (or on threshold)
- Single Postgres transaction per flush:
  1. Positions, accounts, insurance fund: row `UPSERT` (no version
     guard -- advisory lock guarantees single writer per shard)
  2. Fills, funding, liquidations: row `INSERT` (append-only)
  3. Tips: `UPSERT` (no guard -- single writer)
- `synchronous_commit = on` for durability

### Backpressure

Per WAL.md:
- Persistence ring full -> **must stall** hot path (ME stalls upstream)
- Flush lag > 10ms -> **must stall** hot path
- Replica ring full -> **must stall** hot path (configurable, 100ms bound)

## Replication & Failover

**Failover guarantees:** See [GUARANTEES.md](../../GUARANTEES.md) for complete
specification of failover behavior, data loss bounds (single crash: 10ms,
dual crash: 100ms), and recovery time objectives.

**Failover procedures:** See [RECOVERY-RUNBOOK.md](../../RECOVERY-RUNBOOK.md)
§2.2 for step-by-step recovery from Risk master crash, and §4 for dual
crash scenarios.

### Warm standby

Every process is an identical warm candidate; there is no replica
role and no tip-sync stream from the main.

1. Connect Postgres, run migrations, load positions, accounts, tips,
   frozen orders, insurance fund and the active symbol config —
   **without** the advisory lock
2. Open each ME's replication stream from `tips[symbol_id] + 1`
   and apply every record through `replay::apply_record` (the same
   path live gap recovery uses); drain mark prices meanwhile
3. Caught up ⟺ `RECORD_CAUGHT_UP { live_seq }` seen on every
   stream and `applied_seq >= live_seq`
4. Only then: `pg_try_advisory_lock(shard_id)`, non-blocking,
   retried every `RSX_RISK_LEASE_POLL_MS` (default 500ms)

### Promotion (warm -> live)

1. Lock acquired (the main's connection dropped, or it never ran)
2. Final drain: apply records written between the last
   `CAUGHT_UP` and the lock grant (promotion invariant: nothing is
   skipped, nothing applied twice — the tip dedup makes it idempotent)
3. Reload symbol config (versions applied while warm)
4. Start the lease-renewal thread and the write-behind worker
5. Bind the gateway receiver and senders, run the live loop

### Demotion (live -> warm)

Lease loss, lease-check errors, persist-worker death: the live loop
returns, both sidecars are stopped (the worker flushes what it
holds), and the node re-enters warm standby. SIGTERM exits the
process instead of re-entering.

**Implementation:** `app::run` calls `app::run_node` in a loop;
`run_node` walks `promotion::NodeState` (`WarmCatchup` → `Live`)
through `promotion::run_warm_catchup` and `cast_loop::run`, which
reports why it stopped as `cast_loop::LiveExit`. Pinned by
`rsx-risk/tests/replication_e2e_test.rs` and
`rsx-risk/tests/crash_guarantees_test.rs`.

### Recovery: Both Crash

**Data loss bound:** 100ms positions (worst case if both crash before Postgres
flush). Fills are NEVER lost — ME WAL retains all fills for 10min, Risk replays
from `tips[symbol_id] + 1`.

1. New instance reads positions + tips from Postgres (up to 10ms stale)
2. Requests replay via replication consumer ([10-replication.md](10-replication.md)):
   `from_seq = tips[symbol_id] + 1`
3. MEs serve from WAL retention (10-replication.md section 2)
4. Replays to current, acquires the advisory lock, goes live

**Idempotent replay:** Fill processing is idempotent — replaying
a fill with `seq <= tips[symbol_id]` is a no-op (dedup by seq).
Tip persistence is an optimization (reduces replay window). Even
if tip is stale, `position = sum(fills)` is always rebuildable
from ME WAL. The system converges to correct state regardless of
tip staleness.

**100ms loss bound proof:** Risk flushes to Postgres every 10ms. If both
instances crash before flush, max loss = 10ms of position updates. If Postgres
ALSO slow to commit (transaction in progress), max loss extends to 100ms. After
recovery, all positions are reconstructed from ME fills (position = sum(fills)).

### Matching Engine Failover

- ME replica starts sending (lease-based authority)
- ME main shuts up when it detects replica's authoritative stream
- Risk engine deduplicates by `(symbol_id, seq)` -- no restart
  needed

## Main Loop Pseudocode

Priority order: (1) fills from all MEs — never skip, stash BBOs for later;
(2) new orders from gateway — never skip; (3) mark prices; (4) BBO price updates
— skippable under load, only process latest per symbol, triggers margin recalc;
(5) funding settlement every 8h; (5.5) liquidation processing; (6) lease renewal ~1s.

See `rsx-risk/src/cast_loop.rs` and `rsx-risk/src/shard.rs::tick`.

## File Organization

```
rsx-risk/src/
    main.rs           -- entrypoint: panic handler, tracing, app::run
    app.rs            -- composition root: restart loop, run_node
    cast_loop.rs      -- pinned live loop over the cast sockets
    promotion.rs      -- NodeState, warm catch-up, lock acquisition
    shard.rs          -- RiskShard state, set_state, tick dispatcher
    position.rs       -- Position, apply_fill, process_fill
    account.rs        -- Account, collateral
    margin.rs         -- PortfolioMargin, process_order, freeze lifecycle
    price.rs          -- IndexPrice (size-weighted mid), mark/BBO handling
    funding.rs        -- rate calc, zero-sum settlement
    liquidation.rs    -- LiquidationEngine, liquidation checks (13-liquidator.md)
    insurance.rs      -- Insurance fund, socialized loss
    lease.rs          -- Advisory lock acquire/renew/release, lease thread
    persist.rs        -- Write-behind worker, Postgres batching, config reload
    replay.rs         -- Cold start from Postgres, apply_record, gap replay
    config.rs         -- env config, symbol config from Postgres
    schema.rs         -- Postgres migrations
    db.rs             -- Postgres connect helper
    rings.rs          -- egress rings, forward_to_gw
    types.rs          -- FillEvent, BboUpdate, RejectReason
    metrics.rs        -- health snapshot
    risk_utils.rs     -- helpers
```

## Performance Targets

| Path | Target | Operations |
|------|--------|------------|
| Fill processing | <1us | HashMap lookup, arithmetic, ring push |
| Pre-trade check | <5us | portfolio margin across all positions |
| Per-tick margin | <10us/user | recalc all exposed users on tick |
| BBO -> index price | <100ns | arithmetic only |
| Postgres flush | every 10ms | batched UPSERT + COPY |
| Failover detection | ~500ms | advisory lock poll interval |
| Replay catch-up | <5s | depends on gap size |

## Implementation Phases

Each phase is standalone, demonstrable, and benchmarkable.

### Phase 1: Position + Margin Math (no I/O)

Pure functions for position tracking, portfolio margin, index price,
and funding rate. No rings, no Postgres, no network.

**Demo:** CLI binary that takes a sequence of fills from stdin (JSON),
prints position state and margin after each fill.

**Files:** `position.rs`, `account.rs`, `margin.rs`, `price.rs`,
`funding.rs`, `types.rs`

### Phase 2: Fill Ingestion + Main Loop (mocked rings)

RiskShard with main loop processing fills, orders, and BBO from
mocked casting/UDP links. No Postgres. State in-memory only.

**Demo:** Binary that creates a shard with mocked ME producers.
Producers generate random fills. Shard processes, prints stats
(fills/sec, margin recalcs/sec, position count).

**Files:** `shard.rs`, `rings.rs`, `config.rs`

### Phase 3: Persistence (testcontainers Postgres)

Write-behind worker persisting positions, fills, and tips to
Postgres. Recovery: cold start from Postgres.

**Demo:** Binary that processes fills, flushes to Postgres, crashes
(kill -9), restarts, loads state from Postgres, verifies positions
match pre-crash state (within 10ms bounded loss).

**Files:** `persist.rs`, `replay.rs`, migrations

### Phase 4: Replication + Failover

Main/replica pair with fill buffering, tip sync, promotion, and
advisory lock lease management.

**Demo:** Start main + replica. Feed fills. Kill main (kill -9).
Observe replica promotes, continues processing. Show no fill loss.
Start new replica for the promoted main.

**Files:** `promotion.rs`, `lease.rs`

### Phase 5: Full System (all components)

Complete risk engine with all components integrated. Multi-symbol,
multi-user, with funding, mark price feed, persistence, replication.

**Demo:** Full system with N mocked matching engines, M users.
Dashboard showing: fills/sec, margin recalcs/sec, Postgres flush
latency, position count, funding rates. Inject price crash ->
observe liquidations.

**Files:** `main.rs`, all integrated

## Tests

Tests: see [TESTING-RISK.md](TESTING-RISK.md) for complete unit
tests, e2e tests, integration tests, smoke tests, benchmarks,
correctness invariants, and test data patterns across all five
implementation phases.
