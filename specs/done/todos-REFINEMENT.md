---
status: shipped
---

# Refinement Backlog
date: 2026-02-22 (updated)

All P0, P1, and P2 items implemented or confirmed already safe.
Remaining: P3 futures and genuine spec/design gaps.

---

## P3 — Future / Spec Gaps

### [mark] Multi-source mark price aggregation unspecified
Only Binance ingestion exists. Need median-of-sources method,
staleness threshold, fallback to index price.
Touches: RISK.md, CONSISTENCY.md, MESSAGES.md (telemetry)

### [mark] Binance feed reconnect details missing from RISK.md
Missing: backoff (1s,2s,4s,8s,max 30s), staleness 10s, stale behavior

### [dxs] CMP protocol: symbol_id sent per-message; should be per-stream
Saves 4 bytes per record; simplifies wire format (handshake/setup frame)

### [dxs] CMP pipeline type layers: Book Event → 4 transform steps
Should be single WAL record. Emit WAL once from book, flow unchanged
to CMP/WAL/consumers.

### [gateway] monoio single-threaded per core; needs work-stealing
Evaluate tokio-uring or glommio; keep io_uring, add work stealing for
connection distribution across many concurrent WS sessions.

### [liquidator] Symbol halt on repeated liquidation failure not implemented
Source: specs/1/13-liquidator.md:347. When liquidation fails repeatedly,
halt symbol trading (spec TODO).

### [future] Stress test targets not validated
1M fills/sec ME, 100K fills/sec DXS replay — run Criterion benchmarks
to confirm or revise GUARANTEES.md numbers.

### [future] Multi-datacenter replication unspecified
Cross-DC lag, partition tolerance, failover behavior not designed.

### [future] Snapshot frequency vs replay time tradeoff not analyzed
More frequent = faster recovery, higher I/O. Need numbers.

### [future] WAL retention vs disk usage worst-case not analyzed
10min retention at peak load — need capacity model.

### [future] smrb crate: shared memory ring buffer
shm_open/mmap backed SPSC, no_std core. Needed for cross-process IPC
beyond same-process rtrb.

### [future] Modify order: v1 deferred
v1: cancel + re-insert. v2: atomic modify-in-place. Not yet designed.

### [liquidation] liquidation_persisted_to_postgres
Requires testcontainers Postgres. Not implemented — needs Docker in CI.
