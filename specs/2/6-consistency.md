---
status: shipped
---

# Consistency — Event Fan-Out

Matching engine produces events into a fixed array buffer. Events fan out
directly to consumers via casting/UDP between processes (SPSC optional in-process).
Matching engine persists its state via WAL + online snapshot, so orderbook
state is recoverable after crash. Positions are persisted at the risk engine
(see [WAL.md](WAL.md)).

**System guarantees:** See [GUARANTEES.md](../../GUARANTEES.md) for formal
specification of consistency model, durability bounds, and recovery guarantees.

## Table of Contents

- [1. Fan-Out: casting/UDP Between Processes](#1-fan-out-cmpudp-between-processes-spsc-optional-in-process)
- [2. Ordering Guarantees](#2-ordering-guarantees)
- [3. Backpressure](#3-backpressure)
- [4. Positions & Risk](#4-positions--risk)
- [5. Crash Behavior](#5-crash-behavior)
- [6. Deferred](#6-deferred)
- [Drain Loop Pseudocode (casting)](#drain-loop-pseudocode-cmp)
- [Key Invariants](#key-invariants)
- [Verification](#verification)

---

## 1. Fan-Out: casting/UDP Between Processes (SPSC Optional In-Process)

```
        Matching Engine
             |
        drain_events()
         /    |    \       \
     [casting]  [casting]  [casting]   [replication]
       |      |      |       |
     Risk  Gateway  MktData    Recorder
```

Matching engine drains `event_buf[0..event_len]` and emits per-consumer
casting/UDP datagrams to Risk, Gateway, and Marketdata. When co-located, an
SPSC ring MAY be used instead, but casting is the default for inter-process
fan-out. A mirrored stream is also emitted to a hot spare matching engine.

Additionally, Recorder instances connect as replication consumers
([replication.md](replication.md) section 8) to archive event streams to daily
files. Recorders are asynchronous — they do not affect the hot path.

Event routing (transport-agnostic):

| Event           | Risk | Gateway | MktData |
|-----------------|------|---------|---------|
| Fill            | yes  | yes     | yes     |
| BBO             | yes  | no      | no      |
| OrderInserted   | no   | no      | yes     |
| OrderCancelled  | no   | yes     | yes     |
| OrderDone       | yes  | yes     | no      |

BBO emitted by ME after any order that changes best bid/ask.
Risk uses it for index price. MktData derives its own BBO from
shadow book.

Fills also update ME-internal position tracking (section 6.5
of ORDERBOOK.md) for reduce-only enforcement. No new event row
needed.

MktData consumer maintains a shadow orderbook per symbol using the
shared `rsx-book` crate. See [MARKETDATA.md](MARKETDATA.md) for
L2 depth, BBO, and trade derivation from these events.

## 2. Ordering Guarantees

- Within a symbol: total order (single-threaded, monotonic `seq`)
- Across symbols: no ordering (independent processes)
- Across consumers: same FIFO order, different processing latency
- Fills precede ORDER_DONE (ref MESSAGES.md)

## 3. Backpressure

- **Gateway ingress** (external): gateway rejects new orders
  with `OVERLOADED` when buffer is full. This is the primary
  user-facing backpressure mechanism.
- **ME SPSC rings** (optional, internal): ring full = matching engine
  **must stall** (bare busy-spin, no `spin_loop()`). This is
  internal backpressure between co-located components.
- **casting/UDP consumers** (default): UDP may drop under load; consumers
  are responsible for gap detection and resubscribe/replay where needed.
- These two layers are independent. Gateway rejection protects
  against external overload; ME stall protects against slow
  consumers.
- Internal rings should be kept small to avoid hiding latency.
- Per-consumer rings — slow market data doesn't stall risk.

## 4. Positions & Risk

- Positions maintained at risk tile only
- Pre-trade margin check before order enters matching
- Check-to-fill window is acceptable — liquidation handles overshoot
- Risk engine persists positions — see [8-database.md](8-database.md)
- No rollback: fills are final

## 5. Crash Behavior

| Component | On crash | Recovery |
|-----------|----------|----------|
| Matching engine | Book lost | Restores from snapshot + WAL replay. |
| Risk engine | Positions persisted | Restarts from persisted state. See 8-database.md. |
| Gateway | User sessions drop | Users reconnect and re-submit. |

**Graceful shutdown:** SIGTERM is treated identically to a
crash. No special shutdown logic (no drain, no flush, no
notification). Recovery handles all state restoration. This
simplifies the codebase — there is exactly one recovery path,
exercised on every restart regardless of cause.

**Detailed crash scenarios:** See [CRASH-SCENARIOS.md](../../CRASH-SCENARIOS.md)
for comprehensive analysis of all failure modes including dual component
crashes, network partitions, and data loss bounds.

**Recovery procedures:** See [RECOVERY-RUNBOOK.md](../../RECOVERY-RUNBOOK.md)
for step-by-step operational recovery procedures.

## 6. Deferred

- Cross-symbol ordering (portfolio margining)

---

## Drain Loop Pseudocode (casting)

```rust
fn drain_events(book: &Orderbook, links: &mut FanOutLinks) {
    for i in 0..book.event_len {
        let event = &book.event_buf[i as usize];

        match event {
            Event::Fill { .. } => {
                links.risk.send_cmp(event);
                links.gateway.send_cmp(event);
                links.market_data.send_cmp(event);
            }
            Event::OrderDone { .. } => {
                links.risk.send_cmp(event);
                links.gateway.send_cmp(event);
            }
            Event::OrderCancelled { .. } => {
                links.gateway.send_cmp(event);
                links.market_data.send_cmp(event);
            }
            Event::OrderInserted { .. } => {
                links.market_data.send_cmp(event);
            }
            Event::BBO { .. } => {
                links.risk.send_cmp(event);
            }
        }
    }
}
```

## Key Invariants

1. Events within a symbol are totally ordered (`seq` monotonic)
2. No cross-symbol ordering
3. All consumers see same event order (casting/UDP preserves order per stream)
4. Matching engine never drops events (ring full = stall)
5. ORDER_DONE is the commit boundary for multi-fill sequences
6. Risk engine persists positions
7. Matching engine persists orderbook via snapshot + WAL

### Cross-reference: CLAUDE.md system-wide invariants

The 10 system-wide invariants in `CLAUDE.md` partially overlap with the
seven above. The mapping (and enforcement point for those not covered
here) is:

- **#1 Fills precede ORDER_DONE.** Covered by §2 ordering rule and by
  invariant 5 above. Enforced by:
  `rsx-book/src/matching.rs::match_at_level` (emits `Event::Fill` before
  any `Event::OrderDone` for the taker) and
  `rsx-matching/src/wal_integration.rs::publish_events` (iterates
  `book.events()` in event-buffer order; each iteration calls
  `wal.append_framed` before fanning the same `Framed` to cast
  destinations, so on-disk and on-wire sequencing matches buffer
  order one-to-one — Fills land in the WAL before the trailing
  ORDER_DONE).
- **#2 Exactly-one completion (ORDER_DONE xor ORDER_FAILED).** Not
  covered here. Enforced by `rsx-book/src/matching.rs::process_new_order`
  — every code path emits exactly one terminal event
  (`OrderFailed` for validation/FOK/reduce-only rejects;
  `OrderCancelled` for post-only crosses; `OrderDone` for IOC residual
  or full fill). Resting orders complete later via cancel or fill.
- **#3 FIFO within price level (time priority).** Not covered here.
  Enforced by `rsx-book/src/book.rs::Orderbook::insert_resting` — new
  orders are linked at `level.tail`; matching walks from `level.head`
  in `match_at_level`.
- **#4 Position = sum of fills.** Not covered here. Enforced by
  `rsx-risk/src/shard.rs::RiskShard::process_fill` calling
  `Position::apply_fill` for both taker and maker on every persisted
  fill (with `seq <= tip` dedup to keep one-to-one with WAL fills).
- **#5 Tips monotonic.** Not covered here. Enforced by
  `rsx-cast/src/replication_client.rs::ReplicationConsumer::run_*`
  (`self.tip = self.tip.max(seq)`) and
  `rsx-risk/src/shard.rs::process_fill` (writes seq after `seq > tip`
  dedup gate). Implied by invariant 1 above on the producer side.
- **#6 Best bid < best ask (no crossed book).** Not covered here.
  Enforced by `rsx-book/src/matching.rs::process_new_order` — incoming
  aggressors consume opposing levels in `match_at_level` until residual
  no longer crosses; only then is the residual inserted via
  `insert_resting`. Post-only orders that would cross are rejected
  before insertion.
- **#7 SPSC preserves event FIFO order.** Covered by invariant 3 above
  (in-process variant). Enforced by `rtrb` (single producer/consumer,
  FIFO ring) at all SPSC sites in `rsx-risk/src/main.rs` and
  `rsx-mark/src/main.rs`.
- **#8 Slab no-leak (allocated = free + active).** Not covered here.
  Enforced by `rsx-book/src/slab.rs::Slab::{alloc,free}` — every
  `alloc()` either pops `free_head` or bumps `bump_next`; every `free`
  pushes back onto `free_head`. Callers (`insert_resting`,
  `unlink_order`, OrderDone path) pair each alloc with one free.
- **#9 Funding zero-sum across users per symbol per interval.** Not
  covered here. Enforced by `rsx-risk/src/funding.rs::calculate_payment`
  — same `(rate, mark)` applied to each user's signed `net_qty`; sum
  over a symbol's users equals `rate * mark * Σ net_qty / 10_000`, and
  `Σ net_qty = 0` by invariant #4 (every fill increments long by qty on
  one side and short by qty on the other).
- **#10 Advisory lock exclusive (one main per shard).** Not covered
  here. Enforced by `rsx-risk/src/lease.rs::AdvisoryLease` using
  Postgres `pg_try_advisory_lock(shard_id)`; replica path in
  `rsx-risk/src/main.rs::run_replica` blocks on `pg_advisory_lock`
  before promotion.

## Verification

- Trace: order -> fill -> drain to 3 rings -> each consumer processes
- Trace: matching engine crash -> empty restart, risk engine positions
  intact
- Trace: risk engine crash -> restart from persisted positions
- Trace: ring full on market data -> matching stalls on that push,
  risk/gateway unaffected
