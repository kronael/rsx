# Consistency — Event Fan-Out

Matching engine produces events into a fixed array buffer. Events fan out
directly to consumers via SPSC rings. No persistence at the matching engine
— it's ephemeral. If the orderbook crashes, it starts empty. Positions are
persisted at the risk engine (see [PERSISTENCE.md](PERSISTENCE.md)).

Risk engine has timestamps of both input and output, so outstanding orders
can potentially be reconstructed from risk engine data. Orderbook
checkpointing may be added later. This document covers event flow only.

---

## 1. Fan-Out: Direct SPSC from Matching Engine

```
        Matching Engine
             |
        drain_events()
         /    |    \
     [SPSC] [SPSC] [SPSC]
       |      |       |
     Risk  Gateway  MktData
```

Matching engine drains `event_buf[0..event_len]` directly into per-consumer
SPSC rings. No persistence tile in the path. Events are emitted per-fill as
they happen.

Event routing:

| Event           | Risk | Gateway | MktData |
|-----------------|------|---------|---------|
| Fill            | yes  | yes     | yes     |
| OrderInserted   | no   | no      | yes     |
| OrderCancelled  | no   | yes     | yes     |
| OrderDone       | yes  | yes     | no      |

## 2. Ordering Guarantees

- Within a symbol: total order (single-threaded, monotonic `seq_no`)
- Across symbols: no ordering (independent processes)
- Across consumers: same FIFO order, different processing latency
- Fills precede ORDER_DONE (ref PROTOCOL.md)

## 3. Backpressure

- Ring full = matching engine stalls (bare busy-spin, no `spin_loop()`)
- Per-consumer rings — slow market data doesn't stall risk
- Ring sizing: 64K slots, 8MB per ring at 128B/event

## 4. Positions & Risk

- Positions maintained at risk tile only
- Pre-trade margin check before order enters matching
- Check-to-fill window is acceptable — liquidation handles overshoot
- Risk engine persists positions — see [PERSISTENCE.md](PERSISTENCE.md)
- No rollback: fills are final

## 5. Crash Behavior

| Component | On crash | Recovery |
|-----------|----------|----------|
| Matching engine | Book lost | Starts empty. Outstanding orders potentially reconstructable from risk engine data (has input/output timestamps). Future: checkpointing. |
| Risk engine | Positions persisted | Restarts from persisted state. See PERSISTENCE.md. |
| Gateway | User sessions drop | Users reconnect and re-submit. |

## 6. Deferred

- Orderbook persistence / WAL (PERSISTENCE.md covers risk engine only)
- Orderbook checkpointing for faster recovery
- Reconstructing book from risk engine data
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
        }
    }
}

// Bare spin, dedicated core
fn push_spin<T>(ring: &mut SpscProducer<T>, item: &T) {
    while ring.try_push(item).is_err() {}
}
```

## Key Invariants

1. Events within a symbol are totally ordered (`seq_no` monotonic)
2. No cross-symbol ordering
3. All consumers see same event order (SPSC = FIFO)
4. Matching engine never drops events (ring full = stall)
5. ORDER_DONE is the commit boundary for multi-fill sequences
6. Risk engine is the only durable state — positions persisted there
7. Orderbook is ephemeral — crash = empty

## Verification

- Trace: order -> fill -> drain to 3 rings -> each consumer processes
- Trace: matching engine crash -> empty restart, risk engine positions
  intact
- Trace: risk engine crash -> restart from persisted positions
- Trace: ring full on market data -> matching stalls on that push,
  risk/gateway unaffected
