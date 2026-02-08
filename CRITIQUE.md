# Critique

Deep audit of all v1 specs. All items resolved.

## Summary

| Severity | Count | Status |
|----------|-------|--------|
| Critical | 9 | Resolved |
| High | 12 | Resolved |
| Medium | 15 | Resolved |

---

## Critical (Resolved)

### C1. IOC/FOK in matching engine
**Resolution:** Implemented IOC/FOK in ORDERBOOK.md section 5.
IOC cancels remainder after match. FOK rejects if not fully
filled. `tif: u8` added to OrderSlot. GRPC.md TimeInForce
enum unchanged (already had IOC=1, FOK=2).

### C2. CaughtUp semantics
**Resolution:** Defined in DXS.md section 5. `live_seq` =
last seq consumer has seen (inclusive). Resume at
`live_seq + 1`. CaughtUp is per-symbol stream, not global.

### C3. Mark price unavailability during liquidation
**Resolution:** Fallback chain in LIQUIDATOR.md section 3:
mark -> index (BBO) -> last known mark price. Liquidation
never stalls. See also MARK.md section 4.

### C4. Dedup window unsafe across ME restarts
**Resolution:** Dedup by `(user_id, order_id)` persisted in
WAL as `RECORD_ORDER_ACCEPTED`. Replay rebuilds dedup set.
See DXS.md section 1.

### C5. No service discovery spec
**Resolution:** Env vars for static config (`RSX_RISK_ADDR`
etc). Optional Consul/DNS for dynamic discovery. Independent
mechanisms. See NETWORK.md "Service Discovery" and DEPLOY.md.

### C6. Clock sync for funding settlement
**Resolution:** NTP required (<100ms skew).
`interval_id = epoch / 8h` as idempotency key. See RISK.md
section 5 and GUARANTEES.md section 8.5.

### C7. Config propagation crash window
**Resolution:** ME writes applied config to Postgres directly.
Consumers bootstrap from DB on cold start. CONFIG_APPLIED
event is optimization for live sync. See METADATA.md and
RISK.md section 1.

### C8. No deployment/ops spec
**Resolution:** DEPLOY.md created. Single-machine dev
topology, env file config with `RSX_` prefixes, v1 capacity
targets.

### C9. Upgrade/schema versioning
**Resolution:** Version policy in DXS.md and WAL.md. Additive
changes: no version bump (consumers ignore trailing bytes).
Breaking changes: bump version + fail-fast.

---

## High (Resolved)

### H1. Reduce-only enforcement is per-symbol only
**Resolution:** Per-symbol is correct and intended. `reduce_only`
means "reduce THIS symbol's position". Cross-symbol reduce-only
is not a v1 requirement. Documented as intended behavior.

### H2. Snapshot-migration mutual exclusion
**Resolution:** Single-threaded main loop serializes access.
Snapshot checks `book.state`; if Migrating, returns early
(no-op). Next snapshot cycle retries. No lock needed. See
ORDERBOOK.md section 2.7.

### H3. Auth state machine incomplete
**Resolution:** Dropped A frame entirely. Auth only via WS
upgrade headers (JWT). Clients that cannot set headers use
gRPC API. See WEBPROTO.md "Authentication".

### H4. Backpressure scope clarified
**Resolution:** Two independent layers documented. Gateway
rejects at ingress (OVERLOADED) for external traffic. ME
stalls on internal SPSC rings for co-located consumers. See
CONSISTENCY.md section 3 and NETWORK.md.

### H5. Tip persistence vs crash recovery
**Resolution:** Fill replay is idempotent. Tip is optimization
to reduce replay window. `position = sum(fills)` is always
rebuildable regardless of tip staleness. See RISK.md
"Recovery: Both Crash".

### H6. Graceful shutdown not specified
**Resolution:** SIGTERM = same as crash. No special shutdown
logic. Recovery handles all state restoration. Single recovery
path for all restart causes. See CONSISTENCY.md section 5.

### H7. Startup ordering dependencies
**Resolution:** Any boot order. Components retry connecting to
deps with exponential backoff (1s-30s). Gateway rejects until
Risk ready. See NETWORK.md "Startup Ordering".

### H8. Malformed WS frame handling
**Resolution:** Send `{E:[code, msg]}` on parse error. Don't
close connection unless fatal (binary frame, oversized message,
auth failure). See WEBPROTO.md "E: Error".

### H9. Liquidation fill vs round escalation race
**Resolution:** Single-threaded risk loop eliminates race. Fills
and liquidation processing are serialized in the main loop. No
concurrent reads of partial state. See RISK.md section 7.

### H10. Post-max-rounds behavior
**Resolution:** After max_rounds, continue at 100% slippage
(max_slip_bps). If position cannot close, socialized loss via
insurance fund. See LIQUIDATOR.md section 9.

### H11. Recovery snapshot offset
**Resolution:** Replay from `snapshot_seq + 1` (exclusive).
Snapshot includes all state up to and including `snapshot_seq`.
See ORDERBOOK.md section 2.8.

### H12. WS/gRPC field mapping
**Resolution:** Explicit field mapping table added to
WEBPROTO.md. B snapshot: first array = bids (descending),
second = asks (ascending).

---

## Medium (Resolved)

### M1. Fee rounding direction
**Resolution:** Floor always (integer division truncation).
Exchange keeps the sub-tick remainder. See RISK.md section 1.

### M2. Best bid/ask tracking during migration
**Resolution:** Same as H2. Snapshot skips if migration active.
Best-effort, next cycle catches it. Single-threaded access
prevents inconsistency.

### M3. User state cleanup grace period
**Resolution:** Fixed 300s grace period. Disabled during WAL
replay (reclamation deferred until live). See ORDERBOOK.md
section 6.5.

### M4. Config distribution 10min lag
**Resolution:** Accepted tradeoff. 10min lag documented.
Orders matched with stale config in the window are valid
(config changes are infrequent, effects are minor).

### M5. Cancel cid/oid ambiguity
**Resolution:** cid is fixed 20-char string, zero-padded. oid
is UUIDv7 (16 bytes on wire, 32-char hex in WS). Server
distinguishes by length. See WEBPROTO.md and GRPC.md.

### M6. NETWORK_ERROR not in FailureReason
**Resolution:** Added NETWORK_ERROR=8, RATE_LIMIT=9,
TIMEOUT=10 to GRPC.md FailureReason enum. Referenced in
RPC.md.

### M7. Capacity planning absent
**Resolution:** v1 targets documented in DEPLOY.md: 1 ME per
symbol, 1 Risk shard, <10K users, <10 symbols.

### M8. SPSC ring sizing rationale
**Resolution:** Ring sizing = peak_throughput * 2 headroom.
Documented in DEPLOY.md with per-ring capacity table.

### M9. Advisory lock edge cases
**Resolution:** Postgres advisory locks are per-connection.
Auto-released on disconnect (crash, network failure). No
stale lock cleanup needed. See GUARANTEES.md section 8.7.

### M10. Liquidation order in smooshed zone
**Resolution:** Accepted tradeoff. Smooshed zone scan is O(k)
per slot but only reached during extreme moves. Round
escalation handles unfilled orders from smooshed zones.

### M11. Replica replication protocol
**Resolution:** Standard gRPC streaming (same DXS replay
protocol used by all consumers). Documented in NETWORK.md.

### M12. Health check endpoints
**Resolution:** `/health` endpoint spec in DEPLOY.md. JSON
response: `{status, seq, version, uptime_sec}`. 200=ok,
503=not ready.

### M13. WAL rotation crash recovery
**Resolution:** Append-only WAL. Active file uses temporary
name. Crash mid-rotation: reader treats `_active.wal` as last
file, CRC truncates at first invalid record. See WAL.md.

### M14. Heartbeat collision handling
**Resolution:** Client echoes `{H:[ts]}` with its own
timestamp. Simultaneous heartbeats harmless -- no sequence
needed, each side tracks own timeout. See WEBPROTO.md.

### M15. Liquidation slippage accounting
**Resolution:** Slippage = realized_pnl impact. Normal fill
accounting applies -- the liquidation fill price includes
slippage, which flows through standard PnL calculation.

---

## Accepted Tradeoffs

These are known limitations, not bugs:

- **Ingress orders can be lost.** Orders at gateway ingress are
  not WAL'd. On risk crash, in-flight orders are lost. Users
  must resubmit. (CONSISTENCY.md section 5)

- **Backpressure correctness depends on strict stalling.** Ring
  full = ME busy-spins. If stalling is incorrect, data loss.
  (WAL.md "Hard Backpressure")

- **UTC scheduling depends on clock sync.** Funding, config
  effective_at_ms, and staleness sweeps all assume NTP <100ms.
  (See C6 resolution)

- **Check-to-fill margin window.** Pre-trade check uses mark
  price that may be 1-2 ticks stale. Liquidation handles
  overshoot. (CONSISTENCY.md section 4)

- **10ms position loss on dual crash.** Risk flushes to Postgres
  every 10ms. Both instances crashing before flush = 10ms of
  position updates lost. Fills are never lost (ME WAL).
  (GUARANTEES.md section 3.2)
