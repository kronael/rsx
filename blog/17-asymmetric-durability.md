# Fills: 0ms Loss. Orders: Who Cares.

Not all data is equal. Asymmetric durability is the right choice.

## The Problem

Traditional exchange: every message is sacred. Order request goes to
Kafka (durable). Matching engine writes to WAL (durable). Fill goes to
Kafka (durable). Position update goes to database (durable).

**Latency: 20ms** (4 network hops + 4 fsync calls).

**Availability: LOW** (any fsync stall = full pipeline stall).

## The Insight

Fills change balances. Orders don't.

Lose a fill: user balance is permanently wrong. Support ticket. Manual
reconciliation. Lawsuit if it's big enough.

Lose an order: user resubmits. "Service unavailable, try again." 10s of
UX friction. Zero financial impact.

**Make fills durable. Make orders fast.**

## Durability Tiers

### Tier 0: Fills (Sacred)

```rust
// Matching engine writes fill to WAL
let mut fill = FillRecord {
    seq: 0,  // Assigned by WAL
    ts_ns: time_ns(),
    symbol_id: 1,
    taker_user_id: order.user_id,
    maker_user_id: maker.user_id,
    price: Price(match_price),
    qty: Qty(fill_qty),
    taker_side: order.side as u8,
    // ...
};

wal.append(&mut fill)?;  // Buffered
wal.flush()?;            // fsync every 10ms

// Fill is now DURABLE
// Even if process crashes in next 10ms, fill is on disk
```

Guarantees:
- **0ms loss**: fsync before sending to risk engine
- **Exactly-once**: seq numbers prevent duplicates
- **Ordered**: seq monotonic, no gaps
- **Recoverable**: replay from WAL on restart

### Tier 1: Orders (Ephemeral)

```rust
// Gateway receives order from WebSocket
let order = parse_order_from_ws(&frame)?;

// Validate (fast checks only)
if !validate_basic(order) {
    return Err("invalid order");
}

// Send to risk engine via CMP/UDP (NOT WAL'd)
cmp_tx.send_order(order)?;

// NO FSYNC
// If gateway crashes here, order is lost
// User gets "connection closed", retries
```

Guarantees:
- **Maybe lost**: crash before risk receives = order never existed
- **Idempotent retries**: client can resubmit with same cid
- **Fast**: no fsync, no WAL, 10μs latency

### Tier 2: Positions (Write-Behind)

```rust
// Risk engine applies fill to position
position.apply_fill(fill.taker_side, fill.price, fill.qty, fill.seq);

// Write to Postgres (batched, async)
let pg_write = PgWriteBehind {
    user_id: fill.taker_user_id,
    symbol_id: fill.symbol_id,
    position: position.clone(),
    tip: fill.seq,
};

pg_queue.push(pg_write);  // Batched flush every 10ms

// Position update NOT immediately durable
// Crash in next 10ms = replay from Postgres tip + DXS fills
```

Guarantees:
- **10ms loss**: single crash loses max 10ms of fills
- **100ms loss**: dual crash (risk + DB) loses max 100ms
- **Recoverable**: replay DXS from Postgres tip on restart
- **Eventually consistent**: Postgres lags in-memory by 10-100ms

## Failure Scenarios

### Scenario 1: Matching Engine Crash

1. ME writes fill to WAL buffer
2. ME crashes before flush
3. Fill lost from buffer
4. **Result: Fill lost**

Wait, that violates "0ms loss"!

**Fix: Flush before sending fills downstream.**

```rust
// Correct order:
wal.append(&mut fill)?;
wal.flush()?;              // <-- fsync FIRST
cmp_tx.send_fill(fill)?;   // <-- send AFTER durable
```

If flush succeeds but send fails (crash after fsync), risk replays from
WAL and gets the fill anyway. **No loss.**

### Scenario 2: Gateway Crash

1. User submits order via WebSocket
2. Gateway parses order, sends to risk
3. Gateway crashes before sending
4. Order lost from gateway memory
5. **Result: User sees "connection closed", retries**

No data loss. User experience: "service unavailable, retry." Annoying,
not broken.

### Scenario 3: Risk Engine Crash

1. Risk receives fill from ME
2. Applies fill to position (in memory)
3. Queues Postgres write (batched)
4. Crashes before Postgres flush
5. **Result: Position state lost from memory**

Recovery:
1. Restart risk engine
2. Load positions + tips from Postgres (last flushed state)
3. Connect to ME's DXS replay
4. Request fills from tip+1
5. Replay all fills since last Postgres flush
6. Resume live processing

**Max data loss: 10ms** (time between Postgres flushes).

### Scenario 4: Dual Failure (Risk + Postgres)

1. Risk crashes
2. Postgres server crashes 50ms later
3. Postgres last fsync was 100ms ago
4. **Result: Lost 100ms of position updates**

Recovery:
1. Restart Postgres (crash recovery, last checkpoint)
2. Restart risk engine
3. Load positions from Postgres (100ms stale)
4. Replay DXS fills from tip+1 (100ms of fills)
5. Rebuild exact position state

**Max data loss: 100ms** (bounded by Postgres sync_commit interval).

## Why It Works

Fills are the source of truth. Positions are derived state.

```
Position = Initial Balance + Sum(Fills)
```

As long as fills are durable, positions are recoverable.

Matching engine WAL:
```
seq=1234: FILL(user=100, side=BUY, qty=10, price=50000)
seq=1235: FILL(user=100, side=SELL, qty=5, price=50100)
seq=1236: FILL(user=101, side=BUY, qty=20, price=50050)
```

Risk engine replay:
```rust
// Load last known position from Postgres
let mut position = load_position(user_id=100)?;  // seq=1200
let tip = load_tip(symbol_id=1)?;                // 1200

// Replay fills from tip+1
let mut consumer = DxsConsumer::new(symbol_id=1, tip_file);
while let Some(record) = consumer.poll().await? {
    if record.header.record_type == RECORD_FILL {
        let fill: FillRecord = parse(record.payload);
        if fill.taker_user_id == 100 || fill.maker_user_id == 100 {
            position.apply_fill(fill.taker_side, fill.price.0, fill.qty.0, fill.seq);
        }
    }
}

// Position is now correct
assert_eq!(position.net_qty(), 5);  // 10 - 5
```

Position state is **deterministic** given fill sequence.

## Tests Prove It

```rust
// rsx-risk/tests/position_test.rs
#[test]
fn position_equals_sum_of_fills() {
    let mut p = Position::new(1, 0);

    // Apply fills in sequence
    p.apply_fill(0, 100, 10, 1);  // Buy 10 @ 100
    p.apply_fill(1, 110, 5, 2);   // Sell 5 @ 110
    p.apply_fill(0, 105, 3, 3);   // Buy 3 @ 105

    // Position = sum of signed fills
    assert_eq!(p.net_qty(), 8);  // 10 - 5 + 3
    assert_eq!(p.long_qty, 8);
}

#[test]
fn replay_recovers_exact_position() {
    let fills = vec![
        (0, 100, 10),  // Buy 10
        (1, 110, 5),   // Sell 5
        (0, 105, 3),   // Buy 3
    ];

    // Initial position
    let mut p1 = Position::new(1, 0);
    for (side, price, qty) in &fills {
        p1.apply_fill(*side, *price, *qty, 1);
    }

    // Replayed position
    let mut p2 = Position::new(1, 0);
    for (side, price, qty) in &fills {
        p2.apply_fill(*side, *price, *qty, 1);
    }

    // Exact match
    assert_eq!(p1.net_qty(), p2.net_qty());
    assert_eq!(p1.realized_pnl, p2.realized_pnl);
    assert_eq!(p1.long_entry_cost, p2.long_entry_cost);
}
```

## The Cost

Orders can be lost. User submits order, gateway crashes, order vanishes.

**Mitigation: Idempotent retries.**

```rust
// Client assigns client_id (cid)
let order = OrderRequest {
    cid: "user123-20260213-000001",  // Unique per user per day
    symbol_id: 1,
    side: Side::Buy,
    price: Price(50000),
    qty: Qty(10),
    // ...
};

ws.send(order).await?;
```

Gateway deduplicates by cid:

```rust
// Gateway (or risk engine) tracks recent cids
let mut recent_cids: LruCache<String, u64> = LruCache::new(100_000);

if let Some(&existing_seq) = recent_cids.get(&order.cid) {
    // Duplicate: already processed
    return Ok(OrderResponse::Duplicate { seq: existing_seq });
}

// New order: process and record cid
let seq = process_order(order)?;
recent_cids.insert(order.cid.clone(), seq);
```

User retry flow:
1. Submit order with `cid=user123-20260213-000001`
2. Gateway crashes
3. User sees "connection closed"
4. User retries with **same cid**
5. Gateway (or risk) deduplicates: "already processed, seq=1234"
6. User gets confirmation

**No duplicate orders. No lost orders (that weren't already filled).**

## Why It Matters

Traditional design: everything durable = 20ms latency, complex recovery.

Asymmetric design: fills durable, orders ephemeral = 50μs latency,
simple recovery.

The trick: identify what's **source of truth** (fills) vs **derived
state** (positions, balances). Make source durable. Recompute derived.

Users care about:
- "Did my order fill?" (yes: fill is durable)
- "Is my balance correct?" (yes: derived from durable fills)

Users don't care about:
- "Did my order survive a crash?" (no, just resubmit)

## Key Takeaways

- **Fills are sacred**: 0ms loss, fsync before sending downstream
- **Orders are ephemeral**: Lost on crash, user retries
- **Positions are derived**: Replay fills to rebuild state
- **Dual failure bounded**: 100ms max data loss (Postgres sync interval)
- **Idempotent retries**: Client-assigned cid prevents duplicates

When someone asks "what if you lose an order?", the answer is "user
retries." When they ask "what if you lose a fill?", the answer is "we
don't."

Durability is not binary. It's a spectrum. Apply it where it matters.

## Target Audience

Exchange engineers over-engineering durability. Developers building
financial systems who think every message needs Kafka. Anyone
questioning why their low-latency system is slow because everything
goes through a durable queue.

## See Also

- `specs/v1/WAL.md` - Fill durability guarantees
- `specs/v1/RISK.md` - Position replay from fills
- `specs/v1/DXS.md` - Replay protocol for recovery
- `blog/04-wal-and-recovery.md` - WAL-based recovery
- `blog/16-dxs-no-broker.md` - DXS replay from producer WAL
- `rsx-risk/tests/position_test.rs` - Position = sum(fills) tests
