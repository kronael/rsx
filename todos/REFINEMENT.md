# Refinement Backlog
date: 2026-02-22 (updated)

All P0 and P2 items implemented or confirmed already safe.
Remaining: P3 futures and genuine spec gaps.

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
Source: specs/v1/LIQUIDATOR.md:347. When liquidation fails repeatedly,
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

---

## P1 — Spec Test Gaps (not yet written)

### Gateway (TESTING-GATEWAY.md)
- heartbeat_sent_every_5s
- heartbeat_timeout_closes_at_10s
- heartbeat_client_response_resets_timer
- symbol_not_found_rejects_early
- config_cache_updated_on_config_applied
- ws_new_order_accepted_and_filled (E2E)
- concurrent_sessions_isolated (E2E)
- fills_precede_order_done_on_wire
- liquidation_order_routed_correctly (E2E)
- circuit_breaker_opens_on_gateway_overload (E2E)
- rate_limit_per_user_enforced_e2e

### Risk Engine (TESTING-RISK.md)
- order_while_user_liquidated_rejected
- config_applied_event_updates_params
- config_applied_forwarded_to_gateway
- main_lease_acquired_at_startup
- replica_promoted_on_main_failure
- fill_buffering_during_promotion
- crash_recovery_replays_from_tip
- full_lifecycle_order_to_settlement
- liquidation_cascade_multiple_users
- me_failover_dedup_preserved
- funding_settlement_all_intervals

### Liquidator (TESTING-LIQUIDATOR.md)
- liquidate_largest_position_first
- partial_liquidation_reduces_to_target
- multiple_symbols_liquidated_independently
- new_orders_rejected_during_liquidation
- price_drops_triggers_liquidation (E2E)
- cascade_liquidation_across_users (E2E)
- liquidation_persisted_to_postgres (E2E)
- recovery_resumes_pending_liquidations (E2E)
- order_failed_retries_with_slip (E2E)
- insurance_fund_absorbs_deficit (E2E)
- symbol_halt_on_repeated_failure (E2E)
