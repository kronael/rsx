# Consistency — Event Fan-Out

Matching engine produces events into a fixed array buffer. Events fan out
directly to consumers via SPSC rings. Matching engine persists its state via
WAL + online snapshot, so orderbook state is recoverable after crash. Positions
are persisted at the risk engine (see [WAL.md](WAL.md)).

**System guarantees:** See [GUARANTEES.md](../../GUARANTEES.md) for formal
specification of consistency model, durability bounds, and recovery guarantees.

---

## 1. Fan-Out: Direct SPSC from Matching Engine

```
        Matching Engine
             |
        drain_events()
         /    |    \       \
     [SPSC] [SPSC] [SPSC]  [DXS]
       |      |       |       |
     Risk  Gateway  MktData      Recorder
                    (MARKETDATA.md)
```

Matching engine drains `event_buf[0..event_len]` directly into per-consumer
SPSC rings *within the same process*. Events are emitted per-fill as they
happen. A mirrored stream is also emitted to a hot spare matching engine.

Additionally, Recorder instances connect as DXS consumers
([DXS.md](DXS.md) section 8) to archive event streams to daily
files. Recorders are asynchronous — they do not affect the hot path.

Event routing:

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
- Fills precede ORDER_DONE (ref GRPC.md)

## 3. Backpressure

- **Gateway ingress** (external): gateway rejects new orders
  with `OVERLOADED` when buffer is full. This is the primary
  user-facing backpressure mechanism.
- **ME SPSC rings** (internal): ring full = matching engine
  **must stall** (bare busy-spin, no `spin_loop()`). This is
  internal backpressure between co-located components.
- These two layers are independent. Gateway rejection protects
  against external overload; ME stall protects against slow
  consumers.
- Internal rings should be kept small to avoid hiding latency.
- Per-consumer rings — slow market data doesn't stall risk.

## 4. Positions & Risk

- Positions maintained at risk tile only
- Pre-trade margin check before order enters matching
- Check-to-fill window is acceptable — liquidation handles overshoot
- Risk engine persists positions — see [PERSISTENCE.md](PERSISTENCE.md)
- No rollback: fills are final

## 5. Crash Behavior

| Component | On crash | Recovery |
|-----------|----------|----------|
| Matching engine | Book lost | Restores from snapshot + WAL replay. |
| Risk engine | Positions persisted | Restarts from persisted state. See PERSISTENCE.md. |
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

## Drain Loop Pseudocode

```rust
fn drain_events(book: &Orderbook, links: &mut FanOutLinks) {
    for i in 0..book.event_len {
        let event = &book.event_buf[i as usize];

        match event {
            Event::Fill { .. } => {
                links.risk.push_spin(event);
                links.gateway.push_spin(event);
                links.market_data.push_spin(event);
            }
            Event::OrderDone { .. } => {
                links.risk.push_spin(event);
                links.gateway.push_spin(event);
            }
            Event::OrderCancelled { .. } => {
                links.gateway.push_spin(event);
                links.market_data.push_spin(event);
            }
            Event::OrderInserted { .. } => {
                links.market_data.push_spin(event);
            }
            Event::BBO { .. } => {
                links.risk.push_spin(event);
            }
        }
    }
}

// Bare spin, dedicated core
fn push_spin<T>(ring: &mut SpscProducer<T>, item: &T) {
    while ring.try_push(item).is_err() {}
}
```

## Key Invariants

1. Events within a symbol are totally ordered (`seq` monotonic)
2. No cross-symbol ordering
3. All consumers see same event order (SPSC = FIFO)
4. Matching engine never drops events (ring full = stall)
5. ORDER_DONE is the commit boundary for multi-fill sequences
6. Risk engine persists positions
7. Matching engine persists orderbook via snapshot + WAL

## Verification

- Trace: order -> fill -> drain to 3 rings -> each consumer processes
- Trace: matching engine crash -> empty restart, risk engine positions
  intact
- Trace: risk engine crash -> restart from persisted positions
- Trace: ring full on market data -> matching stalls on that push,
  risk/gateway unaffected
