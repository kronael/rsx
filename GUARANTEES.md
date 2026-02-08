# RSX System Guarantees

Formal specification of consistency, durability, and recovery guarantees for the
RSX perpetuals exchange. This document defines what the system promises under
various failure scenarios.

**Key Principles:**
- Fills have 0ms data loss guarantee (replay path exists)
- Orders have loose guarantee (can be lost without user notification)
- Single component failure: 0ms loss (redundancy covers)
- Dual component failure: bounded 100ms loss acceptable
- Backpressure enforced (never drop data silently)

Cross-references: [specs/v1/CONSISTENCY.md](specs/v1/CONSISTENCY.md),
[specs/v1/WAL.md](specs/v1/WAL.md),
[specs/v1/RISK.md](specs/v1/RISK.md),
[specs/v1/DATABASE.md](specs/v1/DATABASE.md),
[specs/v1/DXS.md](specs/v1/DXS.md)

---

## 1. Consistency Model

### 1.1 Fills: Exactly-Once Semantics

**Guarantee:** Once a fill is emitted by the matching engine, it is persisted in
WAL within 10ms and can be replayed indefinitely. Fills are NEVER lost.

**Mechanism:**
- ME writes fill to WAL buffer (in-memory)
- WAL flush every 10ms OR 1000 records (whichever comes first)
- fsync enforced (synchronous disk write)
- DXS replay server retains WAL for 10min minimum
- Risk deduplicates fills by `(symbol_id, seq)` on replay
- Fills are idempotent (replaying same fill = no position change)

**Verification:**
- After any crash/recovery: `sum(fills in Postgres) = sum(fills in ME WAL)`
- Position = sum of fills (invariant verified on every test)

### 1.2 Positions: Eventually Consistent

**Guarantee:** Positions are eventually consistent with fills, bounded by 10ms
flush interval. After any crash, positions can be reconstructed from ME fills
via DXS replay.

**Mechanism:**
- Risk applies fills immediately to in-memory positions
- Positions written to Postgres via write-behind (10ms batch)
- On crash: replay fills from ME WAL starting from `tips[symbol_id] + 1`
- Fills have total order within symbol (monotonic seq)

**Convergence bound:** 10ms (Postgres flush interval) + replay time (<5s for
typical gap)

**Verification:**
- `position.long_qty = sum(fill.qty where side=buy and fill.seq <= tip)`
- Verified after every recovery scenario

### 1.3 Orders: At-Most-Once Submission

**Guarantee:** In-flight orders can be lost without notification. If gateway
crashes after accepting an order but before risk processes it, order is lost
silently. If risk crashes after accepting but before matching engine receives
it, order is lost silently.

**Rationale:** Order intents are ephemeral. Users must resubmit on timeout.
Fills are the source of truth, not order acceptance.

**Mechanism:**
- Gateway -> Risk: SPSC ring (no persistence)
- Risk -> ME: SPSC ring (no persistence)
- On crash: pending orders in rings are lost
- Gateway dedup window: 5min via UUIDv7 order_id

**User-visible behavior:**
- Order submitted -> gateway returns ack
- Gateway crashes -> in-flight order lost, no error sent
- Risk crashes -> in-flight order lost, no error sent
- Matching engine crashes before WAL flush -> in-flight order lost
- Once fill emitted -> fill NEVER lost

**Verification:**
- Users implement timeout-and-resubmit logic
- Idempotent order submission via UUIDv7 order_id

### 1.4 Market Data: Best-Effort

**Guarantee:** Market data (BBO, L2, trades) can lag or be skipped under load.
No critical state depends on market data delivery.

**Mechanism:**
- MARKETDATA consumes ME events via SPSC ring (lowest priority)
- If ring full: ME stalls (backpressure to gateway)
- Shadow orderbook rebuilt from ME WAL on crash (<1s)

**Verification:**
- MARKETDATA lag under load does not affect trading
- Recovery from ME WAL produces identical shadow book

---

## 2. Durability Guarantees by Component

### 2.1 Matching Engine

**WAL flush interval:** 10ms OR 1000 records

**On crash:**
- Recover from snapshot + WAL replay
- Snapshot taken every 10s (background, no hot-path impact)
- Replay from `snapshot.last_seq + 1` to current WAL tip

**Data loss bound:**
- **Fills:** 0ms (WAL flushed, DXS replay available)
- **In-flight orders:** 10ms window (orders accepted but not yet
  flushed to WAL)

**In-flight orders can be lost:** User submits order -> matching engine
accepts -> matching engine crashes before WAL flush -> order lost, NO
error sent to user. This is acceptable because fills are the source
of truth.

**Fills never lost:** Once emitted to event buffer and flushed to WAL
(within 10ms), fills are durable. Risk can replay from DXS even if
Risk was offline when fill occurred.

**Backpressure triggers:**
- WAL buffer full -> stall order processing
- WAL flush lag > 10ms -> stall order processing
- SPSC ring full (any consumer) -> stall event drain

**Recovery time:** 5-10s (snapshot load + WAL replay)

**Verification:**
- No seq gaps in ME WAL after recovery
- All fills from WAL replayed to Risk
- Orderbook state matches pre-crash state (within 10ms loss window)

### 2.2 Risk Engine

**WAL consumption:** DXS consumer of ME WAL, tracks tip per symbol

**Postgres flush interval:** 10ms OR 1000 records

**On crash:**
- Load positions + tips from Postgres
- Request DXS replay from each ME: `from_seq = tips[symbol_id] + 1`
- Process replay fills (same code path as live)
- On `CaughtUp` for all streams: connect gateway, go live

**Data loss bound:**
- **Fills:** 0ms (ME WAL has replay path, Risk re-applies on startup)
- **Positions:** 10ms (fills received but not yet flushed to Postgres)

**Fills have replay path:** ME retains fills in WAL/DXS for 10min. Risk can
replay from last persisted tip even if Risk was offline for minutes.

**Backpressure triggers:**
- Postgres write-behind lag > 100ms -> stall fill processing
- ME -> Risk SPSC ring full -> stall ME event drain

**Recovery time:** 2-5s (Postgres load + DXS replay)

**Verification:**
- Position = sum of fills (after replay)
- Tips monotonic (never decrease)
- Margin consistent (recalc from scratch = incremental)

### 2.3 Gateway

**Stateless:** No persistence

**On crash:**
- User sessions drop
- Users reconnect
- Pending orders in flight lost (no notification)

**Data loss:** 0ms for trading state (gateway has no critical state)

**Recovery time:** <1s (users reconnect)

**Verification:**
- Users resubmit pending orders on timeout
- Duplicate orders rejected via UUIDv7 dedup (5min window)

### 2.4 Market Data

**On crash:**
- Shadow orderbook lost (ephemeral)
- Rebuild from ME WAL via DXS replay

**Data loss:** 0ms (shadow state, not source of truth)

**Recovery time:** <1s (rebuild from ME WAL)

**Verification:**
- Rebuilt shadow book matches ME orderbook

### 2.5 Postgres

**Configuration:** `synchronous_commit = on`, `fsync = on`

**On crash:**
- Recover from committed transactions
- Postgres WAL guarantees ACID

**Data loss:**
- **Committed transactions:** 0ms
- **Uncommitted batches:** unbounded (up to last batch not committed)

**Risk write-behind pattern:** Batches committed every 10ms, so max loss is
10ms of position updates. These updates are replayed from ME fills on Risk
restart.

**Recovery time:** 30-60s (Postgres recovery process)

**Verification:**
- All committed positions present after recovery
- Uncommitted batches replayed from ME WAL fills

---

## 3. Failure Scenarios & Guarantees

### 3.1 Single Component Failures

#### Gateway Crash

| Property | Value |
|----------|-------|
| Data loss | 0ms (stateless) |
| Recovery time | <1s (reconnect) |
| Effect | Users must resubmit in-flight orders |
| Verification | Users reconnect, trading resumes |

**Procedure:**
1. Users detect connection drop
2. Users reconnect to Gateway
3. Users resubmit pending orders (idempotent via UUIDv7)
4. Gateway routes to Risk as normal

**Guarantees:**
- No fills lost (fills never touch Gateway)
- Orders in flight lost (acceptable per consistency model)

#### Matching Engine Master Crash

| Property | Value |
|----------|-------|
| Data loss | 10ms in-flight orders, **0ms fills** |
| Recovery time | 5-10s (snapshot + WAL replay) |
| Effect | Trading halts for symbol, resumes after recovery |
| Verification | Position = sum(fills), no seq gaps |

**Procedure:**
1. ME heartbeat timeout detected (>5s)
2. ME restarts from snapshot
3. ME replays WAL from `snapshot.last_seq + 1`
4. ME resumes live processing
5. Risk replays any missed fills via DXS

**Guarantees:**
- All fills in WAL preserved (flushed within 10ms)
- In-flight orders accepted but not flushed may be lost (10ms window)
- Risk can replay fills from DXS (10min retention)

#### Risk Engine Master Crash

| Property | Value |
|----------|-------|
| Data loss | 10ms positions |
| Recovery time | 2-5s (Postgres load + DXS replay) |
| Effect | Order submission halts, resumes after recovery |
| Verification | Postgres positions vs ME fills |

**Procedure:**
1. Risk heartbeat timeout detected (>5s)
2. New Risk instance acquires advisory lock
3. Load positions + tips from Postgres
4. Request DXS replay from each ME: `from_seq = tips[symbol_id] + 1`
5. Process replay fills (up to 10min history available)
6. On `CaughtUp`: connect Gateway, go live

**Guarantees:**
- Positions reconstructed from matching engine fills (0ms fill loss)
- Position updates from last 10ms may not be in Postgres yet (replayed
  from matching engine)
- Tips monotonic

#### Postgres Master Crash

| Property | Value |
|----------|-------|
| Data loss | 0ms (committed), 10ms (uncommitted batches) |
| Recovery time | 30-60s (Postgres recovery) |
| Effect | Risk stalls on write-behind lag > 100ms |
| Verification | Check Postgres WAL vs Risk buffer |

**Procedure:**
1. Postgres becomes unavailable
2. Risk write-behind buffer fills up
3. Risk stalls fill processing when buffer lag > 100ms
4. Postgres recovers (or replica promoted)
5. Risk resumes flushing buffer
6. Risk recovers from Postgres + ME WAL replay if restarted

**Guarantees:**
- Committed transactions never lost (Postgres ACID)
- Uncommitted batches (up to 10ms) replayed from ME fills
- Risk backpressure prevents unbounded memory growth

### 3.2 Two Component Failures

#### Matching Engine Master + Replica Crash

| Property | Value |
|----------|-------|
| Data loss | 10ms in-flight orders, **0ms fills** (WAL on both) |
| Recovery time | 10-20s (cold start from snapshot) |
| Effect | Symbol trading halts, DXS replay buffer redundancy |
| Verification | WAL has no gaps, Risk received all fills |

**Scenario:** Both matching engine instances crash within 10ms window
(dual power loss, kernel panic, etc.)

**Procedure:**
1. Both matching engine instances offline
2. New matching engine instance starts from last snapshot
3. Replays WAL from `snapshot.last_seq + 1`
4. Resumes live processing
5. Risk replays from DXS (matching engine WAL available for 10min)

**Guarantees:**
- Fills in WAL never lost (both instances flush same WAL)
- In-flight orders in last 10ms may be lost
- DXS consumers (Risk, market data) replay from last tip

**Data loss bound:** 10ms in-flight orders (not yet in WAL), 0ms fills
(WAL flushed)

#### Risk Master + Risk Replica Crash

| Property | Value |
|----------|-------|
| Data loss | **100ms positions** (both crashed before flush) |
| Recovery time | 30-60s (Postgres load) |
| Effect | Fills replayed from matching engine WAL via DXS |
| Verification | Full position reconciliation |

**Scenario:** Both Risk instances crash within 10ms window (worse: if
Postgres also slow to commit, loss can extend to 100ms)

**Procedure:**
1. Both Risk instances offline
2. New Risk instance acquires advisory lock
3. Loads positions from Postgres (up to 10ms stale)
4. Requests DXS replay from matching engine:
   `from_seq = tips[symbol_id] + 1`
5. Matching engine serves from 10min WAL buffer
6. Risk replays fills, reconstructs positions
7. Resumes live processing

**Guarantees:**
- Fills never lost (matching engine WAL has complete history)
- Position updates from last 10ms-100ms reconstructed from fills
- No position drift (replaying fills is deterministic)

**Data loss bound:** 100ms positions (worst case if Postgres batch
not committed + both Risk instances crashed)

#### Matching Engine + Risk (Same Symbol Shard) Crash

| Property | Value |
|----------|-------|
| Data loss | 10ms in-flight orders, 10ms positions |
| Recovery time | 10-30s (sequential recovery) |
| Effect | Matching engine recovers first, Risk replays |
| Verification | Cross-check WAL vs Risk tips |

**Procedure:**
1. Both matching engine and Risk crash (datacenter power loss)
2. Matching engine recovers from snapshot + WAL
3. Risk recovers from Postgres
4. Risk requests DXS replay from matching engine
5. Matching engine serves from WAL (10min retention)
6. Risk catches up, resumes live

**Guarantees:**
- Matching engine WAL is source of truth for fills
- Risk reconstructs from matching engine fills
- No dependency on order of recovery

#### Matching Engine + Postgres Crash

| Property | Value |
|----------|-------|
| Data loss | 10ms in-flight orders, **100ms positions** |
| Recovery time | 60-120s (DB + matching engine recovery) |
| Effect | Risk stalls until both available |
| Verification | DB recovery + matching engine recovery |

**Procedure:**
1. Both matching engine and Postgres crash
2. Postgres recovers first (or concurrently)
3. Matching engine recovers from snapshot + WAL
4. Risk loads from Postgres (stale positions)
5. Risk requests DXS replay from matching engine
6. Risk catches up, resumes

**Guarantees:**
- Matching engine WAL survives (local disk)
- Postgres may lose uncommitted batches
- Risk replays from matching engine to fill gaps

**Data loss bound:** 10ms in-flight orders, 100ms positions (if
Postgres batch + Risk buffer both not committed)

### 3.3 Three+ Component Failures

#### Matching Engine Master + Replica + Postgres Crash

| Property | Value |
|----------|-------|
| Data loss | 10ms in-flight orders, **100ms positions** |
| Recovery time | 60-180s (full cold start) |
| Effect | Worst case, all persistence lost simultaneously |
| Verification | Snapshot + WAL, Postgres recovery |

**Scenario:** Catastrophic multi-component failure (datacenter power
loss, disk array failure)

**Procedure:**
1. All components crash
2. Postgres recovers (or restores from backup)
3. Matching engine recovers from snapshot + WAL (local or replicated)
4. Risk loads from Postgres
5. Risk replays from matching engine WAL
6. System resumes

**Guarantees:**
- Matching engine snapshot + WAL is ultimate source of truth
- If matching engine WAL disk also lost: restore from offloaded
  archive (Recorder)
- Postgres backup restores committed state

**Data loss bound:** 10ms in-flight orders, 100ms positions (bounded
by WAL flush + DB commit intervals)

#### Risk Master + Replica + Postgres Crash

| Property | Value |
|----------|-------|
| Data loss | **100ms positions** (catastrophic) |
| Recovery time | 120-300s (rebuild from ME WAL) |
| Effect | Must replay all fills from ME WAL history |
| Verification | Position = sum(fills) from genesis |

**Scenario:** All Risk state lost (both instances + Postgres)

**Procedure:**
1. All Risk state lost
2. New Risk instance starts
3. Postgres empty or restored from old backup
4. Risk replays from matching engine WAL from genesis OR last
   Postgres backup
5. Rebuilds all positions from fills
6. Resumes live

**Guarantees:**
- Matching engine WAL is source of truth
- Positions can ALWAYS be reconstructed from fills
- Replay from genesis takes longer but is deterministic

**Data loss bound:** 100ms positions (bounded by last Postgres
commit)

### 3.4 Disk Failures

#### Matching Engine Master Disk Total Failure

| Property | Value |
|----------|-------|
| Data loss | 10ms in-flight orders, **0ms fills** (replica WAL) |
| Recovery time | 20-60s (replica promotion) |
| Effect | Replica becomes master, DXS consumers switch |
| Verification | Replica promotion, DXS switch |

**Procedure:**
1. Matching engine master disk fails (total loss)
2. Matching engine replica promoted to master
3. DXS consumers (Risk, market data) connect to new master
4. New replica started from promoted master's snapshot

**Guarantees:**
- Replica has identical WAL (replicated in real-time)
- DXS consumers replay from tip (no gap)
- Fills never lost

#### Risk Master Disk Total Failure

| Property | Value |
|----------|-------|
| Data loss | **100ms positions** (if replica lag > 10ms) |
| Recovery time | 30-90s (replica promotion + Postgres sync) |
| Effect | Depends on replica lag and Postgres commit state |
| Verification | Replica lag vs Postgres commit |

**Procedure:**
1. Risk master disk fails
2. Risk replica promoted to master
3. Replica state may be 10-100ms behind
4. Replica loads tips from Postgres
5. Replays from matching engine WAL to catch up
6. Resumes live

**Guarantees:**
- Replica buffers fills from matching engine (up to 100ms lag
  acceptable)
- Postgres has committed state
- Replay from matching engine fills any gap

**Data loss bound:** 100ms (if replica lag + Postgres uncommitted
both at max)

#### Postgres Master Disk Failure

| Property | Value |
|----------|-------|
| Data loss | 0ms (synchronous replica) |
| Recovery time | 60-180s (replica promotion) |
| Effect | Requires synchronous replication configured |
| Verification | Synchronous replication config |

**Procedure:**
1. Postgres master disk fails
2. Postgres synchronous replica promoted
3. Risk reconnects to new Postgres master
4. No data loss (synchronous commit guarantees)

**Guarantees:**
- Synchronous replica has all committed transactions
- Risk write-behind resumes after reconnect
- No position loss

**Requirement:** Postgres must be configured with `synchronous_standby_names`

---

## 4. Atomic Operation Guarantees

### 4.1 Order Submission -> Fill -> Position -> Margin

**Not atomic** across components. Best-effort ordering via SPSC rings.

**Sequence:**
1. Gateway receives order (ephemeral)
2. Risk checks margin (in-memory)
3. Risk routes to ME (ephemeral)
4. ME matches, emits fill (durable in WAL within 10ms)
5. Fill drains to Risk via SPSC (or DXS if cross-host)
6. Risk applies fill to position (in-memory)
7. Risk writes position to Postgres (batched 10ms)

**Failure points:**
- Gateway -> Risk: in-flight order lost if gateway crashes
- Risk -> Matching engine: in-flight order lost if risk crashes
- Matching engine -> Risk (fill): never lost (WAL + DXS replay)
- Risk -> Postgres: lost if risk crashes before flush, replayed from
  matching engine fills

**Recovery:** Each component replays from its tip, converges eventually.

### 4.2 Fill Event Fan-Out

Matching engine drains fill events to multiple consumers via SPSC rings:

**Consumers:**
- Risk (position updates)
- Gateway (user notifications)
- MARKETDATA (shadow orderbook)

**Guarantee:** All consumers see same order (FIFO), but may apply at different
times.

**On crash:**
- Risk/Gateway may miss fills in rings (ephemeral)
- Consumers replay from matching engine WAL via DXS
- Matching engine retains WAL for 10min (replay window)

**Verification:**
- Risk position = sum of matching engine fills (dedup by seq)
- MARKETDATA shadow book = matching engine orderbook (rebuilt from WAL)

### 4.3 Position Update -> Margin Calculation -> Liquidation

**Atomic within Risk Engine** (single-threaded per user shard).

**Not atomic** with Postgres persistence (write-behind).

**Sequence:**
1. Risk applies fill to position (in-memory, <1us)
2. Risk recalculates margin (in-memory, <5us)
3. Risk detects liquidation trigger (in-memory)
4. Risk enqueues liquidation (in-memory)
5. Risk writes position to Postgres ring (async, 10ms flush)

**Guarantee:** In-memory state is always consistent. Postgres state may lag by
10ms.

**Recovery:** Recalculate margin from persisted positions on startup. If
Postgres lags, replay fills from matching engine WAL to catch up.

---

## 5. Network Partition Scenarios

### 5.1 Gateway <-> Risk Partition

**Effect:**
- Gateway cannot send orders to Risk
- Gateway detects via timeout (1s)
- Gateway returns error to users (circuit breaker)

**Data loss:** 0ms (orders rejected before acceptance)

**Recovery:** Partition heals → Gateway resumes routing

**Verification:** No orders accepted during partition

### 5.2 Risk <-> Matching Engine Partition

**Effect:**
- Risk cannot receive fills from matching engine
- Matching engine continues matching (buffers fills in WAL/DXS)
- Risk detects via heartbeat timeout

**Recovery:**
- Partition heals
- Risk replays fills from matching engine WAL via DXS
- Risk catches up from `tips[symbol_id] + 1`

**Guarantee:** Fills not lost, but positions lag during partition.

**Data loss:** 0ms (fills buffered in matching engine WAL)

**Partition duration limit:** 10min (matching engine WAL retention,
see specs/v1/DXS.md section 2). If partition lasts >10min, Risk must
rebuild from snapshot + full WAL.

**Verification:** After partition heals, position = sum(fills)

### 5.3 Matching Engine <-> Postgres Partition

**Effect:**
- Matching engine continues running (WAL is local)
- Risk continues if in-memory positions available
- Risk write-behind to Postgres stalls

**Risk:**
- If partition lasts >10min AND Risk crashes: DXS replay buffer expires
- Risk must rebuild from matching engine snapshot + full WAL replay

**Recovery:**
- Partition heals
- Risk resumes flushing to Postgres
- Backlog written in batches

**Verification:** Postgres catches up after partition heals

### 5.4 Risk <-> Postgres Partition

**Effect:**
- Risk continues processing fills (in-memory)
- Postgres write-behind stalls
- When buffer lag > 100ms: Risk stalls fill processing

**Guarantee:** Bounded memory growth (backpressure enforced).

**Recovery:**
- Partition heals
- Risk resumes flushing
- If Risk crashed during partition: replay from matching engine WAL

**Verification:** After partition heals, Postgres = sum(fills)

---

## 6. Backpressure & Stall Guarantees

### 6.1 When Producer Must Stall

**Matching Engine:**
- WAL buffer full (local disk slow)
- WAL flush lag > 10ms (disk write slow)
- SPSC ring full (any consumer slow: Risk, Gateway, market data)

**Risk Engine:**
- Postgres write-behind lag > 100ms (DB slow)
- Matching engine -> Risk SPSC ring full (Risk processing slow)
- Replica sync lag > 100ms (replica slow)

**Effect of Stall:**
- Matching engine stalls on order processing (entire symbol stops)
- Risk stalls on fill processing (user shard stops)
- Gateway rejects new orders (backpressure to users)

**Guarantee:** **Never drop data silently**. Always stall or reject with error.

### 6.2 Backpressure Propagation

```
User -> Gateway -> Risk -> ME
         ^          ^       ^
         |          |       |
      reject    stall    stall
     on timeout on ring  on WAL
                 full     full
```

**End-to-end:** If matching engine cannot keep up, gateway rejects new
orders. Users see explicit rejection, not silent data loss.

### 6.3 SPSC Ring Sizing

**Philosophy:** Keep rings small to avoid hiding latency.

**Typical sizes:**
- Matching engine -> Risk (fills): 4096 entries (targets ~1ms buffering
  at 4M fills/sec)
- Matching engine -> Gateway: 4096 entries
- Matching engine -> market data: 8192 entries (lower priority, can
  lag more)

**Ring full = producer stalls:** Bare busy-spin, no `spin_loop()`,
dedicated core.

**Per-consumer rings:** Slow market data doesn't stall Risk.

---

## 7. Deduplication Windows

| Component | Dedup Key | Window | On Duplicate |
|-----------|-----------|--------|--------------|
| Gateway orders | `order_id` (UUIDv7) | 5min | Reject |
| Risk fills | `(symbol_id, seq)` | Forever | Ignore |
| Risk tips (PG) | `(instance_id, symbol_id)` | Forever | UPSERT |
| PG positions | `(user_id, symbol_id, ver)` | Forever | UPSERT |

**Gateway dedup:** UUIDv7 order_id includes timestamp. Dedup window is
rolling 5min in-memory. After 5min, order_id can be reused (unlikely
with UUIDv7).

**Risk fill dedup:** Tracks `tips[symbol_id]` in-memory. Any fill with
`seq <= tips[symbol_id]` is a duplicate (already applied). On replay
from DXS, dedup prevents double-counting.

**Postgres position dedup:** Version field increments on every update.
UPSERT with version check detects concurrent updates (should never
happen with advisory lock, but defensive).

---

## 8. Invariants Verified on Recovery

After any recovery scenario (crash, partition heal, failover), these
invariants MUST hold. See specs/v1/TESTING-*.md for verification
procedures.

### 8.1 Position = Sum of Fills

For each `(user_id, symbol_id)`:

```
position.long_qty = sum(fill.qty where side=buy
                         and fill.seq <= tips[symbol_id])
position.short_qty = sum(fill.qty where side=sell
                          and fill.seq <= tips[symbol_id])
```

**Verification:** Run reconciliation query on recovery:

```sql
SELECT user_id, symbol_id,
  SUM(CASE WHEN side = 0 THEN qty ELSE 0 END) AS fills_long,
  SUM(CASE WHEN side = 1 THEN qty ELSE 0 END) AS fills_short
FROM fills
WHERE seq <= (SELECT last_seq FROM tips
              WHERE symbol_id = fills.symbol_id
              AND instance_id = ?)
GROUP BY user_id, symbol_id;
```

Compare with positions table. Any mismatch = critical bug.

### 8.2 Tips Monotonic

For each `symbol_id`:

```
tips[symbol_id] >= last_persisted_tips[symbol_id]
```

Tips NEVER decrease. After recovery, tip = last persisted seq OR higher (if
replayed additional fills).

**Verification:** Load tips from Postgres, compare with in-memory tips after
replay. In-memory tips must be >= persisted tips.

### 8.3 Margin Consistent

Recalculate margin from scratch = incremental margin state.

```
margin_from_scratch = calculate_margin(all positions, mark_prices)
margin_incremental = account.equity - account.initial_margin
                     - account.frozen_margin
```

**Verification:** Periodically (every 1000 fills in tests), recalc
margin from scratch and compare with incremental state. Any drift =
bug in margin logic.

### 8.4 No Negative Collateral (Unless Leverage Allowed)

For each `user_id`:

```
account.collateral >= 0
  (or >= -max_leverage * equity if leverage allowed)
```

**Verification:** Query accounts table after recovery, check no
negative collateral (unless user has open positions with unrealized
losses exceeding initial capital).

### 8.5 Funding Zero-Sum

For each `(symbol_id, settlement_interval)`:

```
sum(funding_payments) = 0
```

Longs pay exactly what shorts receive (and vice versa).

**Verification:** After funding settlement, sum all funding payments for that
symbol/interval:

```sql
SELECT symbol_id, settlement_ts, SUM(amount)
FROM funding_payments
GROUP BY symbol_id, settlement_ts;
```

Sum must be 0 (within rounding error of fixed-point arithmetic).

### 8.6 Fills Idempotent

Replaying same fill = no position change.

**Verification:** Process fill twice (in test), verify position unchanged on
second apply (dedup by seq).

### 8.7 Advisory Lock Held

Exactly one main per shard at any time.

**Verification:** Query Postgres `pg_locks`:

```sql
SELECT COUNT(*) FROM pg_locks
WHERE locktype = 'advisory'
AND objid = ?;  -- shard_id
```

Must return exactly 1.

### 8.8 Slab No-Leak (Matching Engine)

For matching engine orderbook slab allocator:

```
allocated_slots = free_slots + active_orders
```

No memory leak in slab.

**Verification:** Track slab stats on every order insert/cancel. After
recovery, verify counts match.

---

## 9. Recovery Time Objectives (RTO)

| Scenario | Best | Typical | Worst |
|----------|------|---------|-------|
| Gateway crash | <1s | <1s | 2s |
| Matching engine master crash | 3s | 5-10s | 20s |
| Risk master crash | 1s | 2-5s | 10s |
| Market data crash | <1s | <1s | 2s |
| Postgres crash | 10s | 30-60s | 120s |
| Matching engine master + replica | 5s | 10-20s | 40s |
| Risk master + replica | 10s | 30-60s | 120s |
| Matching engine + Risk (shard) | 5s | 10-30s | 60s |
| Matching engine + Postgres | 30s | 60-120s | 300s |
| Matching engine + replica + PG | 30s | 60-180s | 600s |
| Risk + replica + Postgres | 60s | 120-300s | 600s |

**Best case:** All components healthy, minimal replay gap, fast disk.

**Typical:** Normal conditions, <1min of replay, healthy hardware.

**Worst case:** Large replay gap (approaching 10min), slow disk,
Postgres vacuum/checkpoint in progress.

---

## 10. Monitoring & Alerting

### 10.1 Metrics to Track

| Metric | Threshold | Alert | Purpose |
|--------|-----------|-------|---------|
| ME WAL flush lag | p99 < 10ms | Warn: p99 > 15ms | Verify 10ms bound |
| ME WAL flush lag | p99 < 10ms | Crit: p99 > 50ms | Detect disk slow |
| Risk Postgres lag | p99 < 10ms | Warn: p99 > 50ms | Verify 10ms bound |
| Risk Postgres lag | p99 < 10ms | Crit: p99 > 100ms | Detect DB slow |
| Risk replica lag | <100ms | Crit: >500ms | Prevent 100ms loss |
| ME seq gaps | 0 | Crit: any gap | Detect lost fills |
| Position delta | 0 | Crit: any delta | Detect mismatch |
| Advisory lock | 1/shard | Crit: 0 or 2 | Detect split-brain |
| SPSC ring full | 0 stalls/s | Warn: >10/s | Detect backpressure |
| DXS replay lag | <1s | Warn: >10s | Detect slow consumer |
| Funding delta | 0 | Crit: >1bps | Detect funding bug |

### 10.2 Alert Severity

**P0 (page immediately):** Data loss risk
- Seq gap detected
- Position reconciliation mismatch
- Advisory lock violated (split-brain or no-main)
- Funding zero-sum violated

**P1 (page during business hours):** Degraded performance
- WAL flush lag > threshold
- Postgres write lag > threshold
- Replica lag > threshold
- SPSC ring stalls

**P2 (email):** Shadow component down
- Replica offline
- Market data offline
- DXS recorder offline

### 10.3 Reconciliation Checks

**Run on recovery:**
- Position = sum(fills) for all users
- Tips monotonic
- Margin recalc from scratch matches incremental
- Advisory lock held exactly once per shard

**Run periodically (every 10min in production):**
- Position reconciliation query (sample 1% of users)
- Margin drift check
- Funding zero-sum check (after settlement)

**Run in tests (every scenario):**
- All invariants verified
- No memory leaks (slab allocator)
- No seq gaps

---

## 11. Data Loss Summary Table

| Failure Scenario | Fills | Orders | Positions | RTO |
|------------------|-------|--------|-----------|-----|
| Gateway crash | 0ms | N/A | 0ms | <1s |
| Matching engine master | **0ms** | 10ms | 0ms | 5-10s |
| Risk master crash | **0ms** | 10ms | 10ms | 2-5s |
| Postgres crash | **0ms** | 0ms | 10ms | 30-60s |
| Market data crash | **0ms** | N/A | 0ms | <1s |
| Matching engine + replica | **0ms** | 10ms | 0ms | 10-20s |
| Risk + replica | **0ms** | 10ms | **100ms** | 30-60s |
| Matching engine + Risk | **0ms** | 10ms | 10ms | 10-30s |
| Matching engine + PG | **0ms** | 10ms | **100ms** | 60-120s |
| Matching engine + rep + PG | **0ms** | 10ms | **100ms** | 60-180s |
| Risk + replica + PG | **0ms** | 10ms | **100ms** | 120-300s |
| Matching engine disk fail | **0ms** | 10ms | 0ms | 20-60s |
| Risk disk failure | **0ms** | 10ms | **100ms** | 30-90s |
| PG disk failure | **0ms** | 0ms | 0ms | 60-180s |

**Key takeaways:**
- **Fills: 0ms loss guarantee** in ALL scenarios (matching engine WAL
  + DXS replay)
- **Orders: 10ms loss acceptable** (ephemeral, users resubmit)
- **Positions: 0-100ms loss** depending on scenario (reconstructed
  from fills)
- **Single component: 0-10ms loss** (redundancy covers)
- **Dual component: 10-100ms loss** (acceptable per requirements)

---

## 12. Testing Strategy

### 12.1 Guarantee Verification

Each guarantee in this document has corresponding tests:

**Unit tests:** Verify deduplication, idempotency, margin calculation

**E2E tests:** Verify fill processing, position updates, WAL replay

**Integration tests:** Verify Postgres persistence, advisory locks, DXS replay

**Chaos tests:** Verify crash scenarios, partition scenarios, backpressure

**Smoke tests:** Verify end-to-end system guarantees in deployed environment

### 12.2 Invariant Verification

All 8 invariants programmatically verified in tests:

1. Position = sum(fills): SQL query after every recovery test
2. Tips monotonic: Assert after replay
3. Margin consistent: Periodic recalc from scratch
4. No negative collateral: Query after recovery
5. Funding zero-sum: Query after settlement
6. Fills idempotent: Double-apply test
7. Advisory lock exclusive: pg_locks query
8. Slab no-leak: Slab stats tracking

### 12.3 Stress Testing

**Sustained load:** 1M fills/sec for 10min, verify no degradation

**Chaos engineering:** Random component kills every 10-60s for 10min

**Partition tests:** Introduce network latency/packet loss for 5min

**Slow disk:** Throttle disk I/O, verify backpressure enforced

**Slow DB:** Throttle Postgres writes, verify bounded lag

**Replay burst:** 100K fills replayed on cold start, verify throughput

---

## 13. Open Questions & Future Work

### 13.1 Quantified Stress Test Targets

**TODO:** Run actual stress tests to validate:
- Matching engine can sustain 1M fills/sec with 10ms WAL flush
- Risk can sustain 1M fills/sec with 10ms Postgres flush
- DXS replay can serve 100K fills/sec to 10 consumers concurrently
- Postgres can handle 100K position updates/sec in batches

### 13.2 Multi-Datacenter Replication

**TODO:** Specify guarantees for geo-distributed deployments:
- Cross-DC latency impact on WAL flush
- Cross-DC replica lag bounds
- Partition tolerance across DC link failure

### 13.3 Snapshot Frequency vs Replay Time

**Current:** Matching engine snapshots every 10s, Risk has no snapshot
(rebuilds from Postgres)

**TODO:** Analyze tradeoff:
- More frequent snapshots = faster recovery, higher I/O overhead
- Less frequent snapshots = slower recovery, lower I/O overhead

### 13.4 WAL Retention vs Disk Usage

**Current:** 10min retention on matching engine WAL, then offload to
Recorder

**TODO:** Analyze:
- Worst-case disk usage for 10min retention
- Replay time if consumer lags >10min (must rebuild from snapshot +
  full WAL)

---

## 14. Summary

**Fills: 0ms loss guarantee**
- Matching engine WAL flushed within 10ms, retained for 10min
- DXS replay available to all consumers
- Idempotent replay prevents double-counting

**Orders: Loose guarantee**
- Can be lost without user notification
- Users resubmit on timeout
- Fills are source of truth

**Positions: 0-100ms loss**
- Single component: 0ms (reconstructed from fills)
- Dual component: 100ms (bounded by flush intervals)
- Always reconstructed from matching engine fills on recovery

**Backpressure enforced**
- Never drop data silently
- Stall or reject with error when overloaded

**Invariants verified**
- Position = sum(fills)
- Tips monotonic
- Margin consistent
- Advisory lock exclusive
- Funding zero-sum

**RTO: 1s - 300s**
- Single component: <10s
- Dual component: <120s
- Catastrophic (3+ components): <300s

This specification is the single source of truth for RSX system
guarantees. All components must adhere to these bounds. Any violation
is a critical bug.
