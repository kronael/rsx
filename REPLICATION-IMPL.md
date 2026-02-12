# Replication/Failover Implementation

Implementation of RISK.md §Replication & Failover for rsx-risk engine.

## Architecture

Two modes per shard:
- **Main**: holds advisory lock, processes orders, persists to Postgres, sends tip sync to replica
- **Replica**: buffers fills from MEs, receives tip sync from main, polls for lock, promotes on acquisition

## Components

### Advisory Lock (lease.rs)
- `AdvisoryLease::acquire(&client)` - blocking acquire (main startup)
- `AdvisoryLease::try_acquire(&client)` - non-blocking (replica polling)
- `AdvisoryLease::renew(&client)` - lease health check
- `AdvisoryLease::release(&client)` - clean shutdown

Postgres advisory lock key = shard_id (i64)

### Replica State (replica.rs)
- `ReplicaState::buffer_fill(fill)` - buffer from ME CMP stream
- `ReplicaState::apply_tip(symbol_id, tip)` - update last known tip from main
- `ReplicaState::drain_fills_up_to_tip(symbol_id)` - apply buffered fills ≤ tip
- `ReplicaState::drain_all_up_to_tips()` - promotion: apply all buffered fills

Buffering strategy:
- `Vec<FxHashMap<u64, FillEvent>>` per symbol (seq-indexed)
- Ordered application via sorted seq on drain
- Idempotent: dedup happens in `process_fill` via tip check

### Main Loop (main.rs)

#### Main Mode (run_main)
1. Acquire advisory lock (blocking)
2. Load positions/tips from Postgres
3. Replay from ME WAL via DXS consumer (tips[symbol_id] + 1)
4. Start persist worker (write-behind to Postgres)
5. Main loop:
   - Process fills/orders/BBO (standard risk logic)
   - Send tip sync to replica via CMP/UDP (record type 0x20)
   - Renew lease periodically (~1s)

#### Replica Mode (run_replica)
1. Try advisory lock (expected to fail, main holds it)
2. Load baseline from Postgres
3. Connect CMP receivers to MEs (same as main)
4. Connect CMP receiver for tip sync from main
5. Replica loop:
   - Buffer fills from MEs into `ReplicaState`
   - Receive tip sync from main, apply buffered fills up to tip
   - Poll advisory lock every ~500ms
   - On lock acquired: promote
6. Promotion:
   - Apply all buffered fills up to last tips (promotion invariant)
   - Restart as main mode via `run_main()`

### Tip Sync Protocol

Message format (record type 0x20):
```rust
#[repr(C, align(64))]
struct TipSyncMessage {
    symbol_id: u32,
    tip: u64,
    _pad: [u8; 48],
}
```

Sent via CMP/UDP from main to replica on every fill.

## Configuration

Environment variables:
- `RSX_RISK_IS_REPLICA=true` - start in replica mode
- `RSX_RISK_REPLICA_ADDR=127.0.0.1:9111` - replica tip sync receiver addr
- `RSX_RISK_CMP_ADDR=127.0.0.1:9101` - main tip sync sender addr
- `RSX_RISK_LEASE_POLL_MS=500` - replica lock poll interval
- `RSX_RISK_LEASE_RENEW_MS=1000` - main lease renewal interval
- `DATABASE_URL` - Postgres connection (required)

## Failover Scenarios

### Main Crash
1. Main process dies, advisory lock released
2. Replica polls, acquires lock (~500ms detection)
3. Replica applies buffered fills up to last tips
4. Replica promotes to main mode
5. Continues processing as main

### Replica Crash
- Main unaffected, continues processing
- Start new replica process
- Replica loads baseline from Postgres
- Replica buffers new fills until tip sync catches up

### Both Crash
1. New instance acquires advisory lock
2. Loads positions/tips from Postgres (up to 10ms stale)
3. Requests replay via DXS consumer from tips[symbol_id] + 1
4. MEs serve from 10min WAL retention
5. Catches up to live, starts new replica

Data loss bound: 100ms positions (worst case, Postgres flush lag). Fills never lost (ME WAL replay).

## Testing

Unit tests (tests/replica_test.rs):
- `replica_buffers_fills_until_tip_received` - basic buffering
- `replica_only_applies_fills_up_to_tip` - promotion invariant
- `replica_promotion_applies_all_buffered` - multi-symbol promotion
- `advisory_lock_acquired_by_main_blocks_replica` - lock exclusivity (Postgres)
- `replica_promotion_after_main_crash` - failover (Postgres)
- `both_crash_recovery_from_postgres` - cold start (Postgres)

Integration tests marked `#[ignore]` require Postgres connection.

Run unit tests:
```bash
cargo test -p rsx-risk --test replica_test
```

Run integration tests:
```bash
DATABASE_URL=postgresql://... cargo test -p rsx-risk --test replica_test -- --ignored
```

## Performance

| Operation | Target | Notes |
|-----------|--------|-------|
| Replica fill buffer | <100ns | HashMap insert |
| Tip sync receive | <1us | CMP/UDP + HashMap lookup |
| Promotion | <1s | Apply buffered fills, depends on buffer size |
| Failover detection | ~500ms | Advisory lock poll interval |

## Future Work

- Streaming replication: continuous tip sync instead of per-fill
- Multi-replica: N replicas per shard for higher availability
- Lease-based authority: replica transitions without restart
- Replica lag monitoring: expose buffered count as metric
