# Bucket 3 findings (specs 25-37, excl. 32)

## 25-process.md

**Status recommendation**: shipped

- §Scope / Terms — match
- §Generic Process Template — bloat (duplicates TILES.md; point to 45-tiles.md)
- §Inter-Process Links — match
- §Telemetry — match ("not implemented in v1" accurate)
- §Gateway — match
- §Risk — drift (omits `insurance.rs`, `rings.rs`, `schema.rs`; lists `fill.rs`, `tip.rs`, `mark_consumer.rs` which don't exist as separate files)
- §Matching / Marketdata / Mark / Recorder / Archive — match
- §Busy-Spin Guidance — match
- §Heartbeats — unshipped (no heartbeat-to-telemetry wiring)

**Actions:**
- Remove tile-list body from §Generic Process Template; replace with "see 45-tiles.md"
- Fix §Risk file list to match actual src/ layout
- Move §Heartbeats to `unshipped` note; link future ship task

## 26-rest.md

**Status recommendation**: partial

- §Overview / /health / /v1/symbols — match (only these two implemented in `rsx-gateway/src/rest.rs:70,74`)
- §GET /v1/account — unshipped
- §GET /v1/positions — unshipped
- §GET /v1/orders — unshipped
- §GET /v1/fills — unshipped
- §GET /v1/funding — unshipped
- §Authentication (JWT) — unshipped (only on WS)
- §Errors / Rate Limits / CORS — unshipped

**Actions:**
- Update frontmatter to partial
- Move unimplemented endpoints + JWT/rate-limit/CORS to `## Deferred` block or specs/3/
- Keep only /health, /v1/symbols, wire-format note in v2 body

## 27-risk-dashboard.md

**Status recommendation**: draft

- §1-9 — all unshipped (no risk ops dashboard anywhere)

**Actions:**
- Change frontmatter to draft
- Move to specs/3/ (not vital for publish MVP)
- OR create ship project if risk ops controls needed pre-publish

## 28-risk.md

**Status recommendation**: shipped (heavy bloat)

- §Architecture Overview — match
- §1 Orderbook Event Ingestion — match
- §2 Position Manager — bloat (struct defs match code)
- §3 Margin Calculator — bloat (structs + formulas duplicated)
- §4 Price Feeds — match
- §5 Funding Engine — match (8h interval, idempotency key)
- §6 Pre-Trade Risk Check — bloat (matches shard.rs)
- §7 Per-Tick Margin Recalc — bloat (matches shard.rs)
- §Persistence Postgres Schema — bloat (belongs in migrations/)
- §Persistence Write Patterns — match
- §Backpressure — match
- §Replication & Failover — match (advisory lock, DXS replay consumer, 500ms poll)
- §Main Loop Pseudocode — bloat
- §File Organization — drift (lists fill.rs/tip.rs/mark_consumer.rs; missing rings.rs/insurance.rs/schema.rs)
- §Performance Targets — match
- §Implementation Phases — bloat (dev-diary content)

**Actions:**
- Strip §2, §3 struct defs — replace with code pointers
- Strip §6, §7, §Main Loop pseudocode — replace with function pointers
- Strip SQL DDL — replace with "see migrations/"
- Fix File Organization to match actual layout
- Strip Implementation Phases

## 29-rpc.md

**Status recommendation**: shipped (heavy bloat)

- §Overview — match
- §Order ID / Why UUIDv7 — match
- §Order ID Generation Point — bloat (duplicates `order_id.rs`)
- §Pending Request LIFO VecDeque — bloat (struct defs)
- §Pending Request FxHashMap — bloat (ME-internal)
- §Request Lifecycle State Machine — match
- §Request Lifecycle 6-step pseudocode — bloat (120 lines duplicating gateway+matching)
- §Bidirectional Stream Protocol — match
- §Error Handling — match
- §Backpressure 10k cap — match
- §Backpressure CMP flow control — match
- §Backpressure Rate limiter impl — bloat (duplicates `rate_limit.rs`)
- §Backpressure Circuit breaker impl — bloat (duplicates `circuit.rs`)
- §Concurrency Tokio task per user — bloat
- §Concurrency ME loop — bloat
- §Performance VecDeque vs HashMap — match
- §Performance Memory Overhead — match
- §Cache Locality — match

**Actions:**
- Delete struct defs and impl blocks (RateLimiter, CircuitBreaker, PendingOrders, flow pseudocode) — replace with code pointers
- Delete 6-step pseudocode — keep only state machine diagram
- Estimated reduction: ~350 → ~100 lines

## 30-scenarios.md

**Status recommendation**: shipped

- §Current State / What Works — match (`build_spawn_plan` etc. all confirmed)
- §Current State / What Is Broken — drift (ALL 5 items now FIXED: RSX_ME_CMP_ADDRS, Binance combined URL, Recorder in build_spawn_plan)
- §Scenario Matrix — match
- §Port Allocation — match
- §Process Spawn Rules — match
- §Implementation Tasks 1-5 — drift (all done, presented as future)
- §Acceptance Criteria — match
- §Testing — match

**Actions:**
- Replace "What Is Broken" with "What Was Fixed"
- Replace §Implementation Tasks 1-5 with "Completed" note + git log reference

## 31-sim.md

**Status recommendation**: FULLY SHIPPED — delete or archive

- §1 Remove Sim Mode — match (`_sim_book` etc. not present in server.py)
- §2 Delete rsx-sim/ — match (doesn't exist)
- §3 Stress Generator — match (`stress.py` exists, endpoints wired)
- §3 Remove in-process stress — match
- §4 Update Docs — partial (unchecked)
- §Acceptance Criteria — all met

**Actions:**
- Delete spec or move to specs/1/ (historical)
- Optionally verify §4 doc updates

## 33-telemetry.md

**Status recommendation**: draft

- §Philosophy — match (direction correct)
- §Emission / structured JSON — drift (processes use `tracing_subscriber::fmt::init()` text, NOT JSON; no `ts_ns` field)
- §Metric Fields per component — unshipped (`match_latency_ns`, `latency_us`, `ring_full_pct` etc. not emitted)
- §Log Transport rsyslog — unshipped (no config, no pipe)
- §Metrics Extraction Vector — unshipped (no vector.toml)
- §System Metrics / Storage Prometheus — unshipped
- §Playground Integration / /api/metrics — match (reads log files)
- §What Processes Do NOT Do — drift (don't emit JSON, drift)
- §Latency Budget — unshipped

**Actions:**
- Change frontmatter to draft
- Fix §Emission to reflect actual `tracing_subscriber::fmt` text logging
- Move rsyslog/Vector/Prometheus sections to specs/3/ as planned production telemetry
- Keep §Philosophy and §Playground Integration

## 34-testing-book.md

**Status recommendation**: shipped (heavy bloat)

- §Requirements B1-B31 — match (most); B15/B29 — drift (claims GTC only in v1, but IOC/FOK/post-only are implemented)
- §Unit Tests (all categories) — match (tests exist); bloat (name lists)
- §E2E Tests — bloat (30-line name list)
- §Benchmarks — match (`book_bench.rs` has all listed)
- §Integration Points — match

**Actions:**
- Fix B15/B29: GTC-only claim is false; IOC, FOK, post-only all implemented
- Strip §Unit Tests / §E2E Tests name lists — replace with file pointers
- Keep Requirements Checklist

## 35-testing-cmp.md

**Status recommendation**: partial (heavy bloat, some unshipped)

- §1 Requirements C1-C45 — match (many `☐` should be `☑`)
- §2.1 Control Message Encoding — match
- §2.2 CmpSender — match
- §2.3 CmpReceiver — match
- §2.4 TCP Replication — match
- §3 E2E Tests — bloat/unshipped (named files `cmp_e2e_test.rs`, `cmp_fault_test.rs` don't exist)
- §4 Benchmarks — match
- §5 Integration Points — match
- §6 monoio Test Considerations — drift (tests use `#[test]` not `#[monoio::test]`)
- §7 Test File Organization — drift (actual: cmp_encoding_test, cmp_test, client_test, header_test, records_test, tls_test, wal_test)
- §8 Coverage Matrix — match

**Actions:**
- Update §7 to match actual test files
- Fix §6 to reflect `#[test]`
- Strip §2, §3 name lists
- Update checklist `☐` → `☑` for implemented items

## 36-testing-dxs.md

**Status recommendation**: shipped (bloat)

- §Requirements D1-D28 — match
- §Unit Tests (all) — bloat (tests exist by name)
- §E2E Tests — bloat (40 entries, correct names)
- §Edge Case Tests — bloat/unshipped (80+ names, many aspirational)
- §Invariant Verification Tests — bloat/unshipped
- §Benchmarks — match
- §Integration Points — match

**Actions:**
- Strip §Unit Tests, §E2E Tests, §Edge Case Tests name lists
- Mark Edge Case / Invariant Tests as `## Aspirational Test Coverage` if not implemented
- Keep Requirements Checklist and Benchmarks

## 37-testing-gateway.md

**Status recommendation**: partial (impl status table outdated)

- §Requirements G1-G31 — match
- §G31 exactly one key — match
- §Unit Tests WS Protocol — match (`protocol_test.rs`)
- §Enum Validation — drift (spec says 0-7; code has 0-12 for FailureReason)
- §Fill Fee / Reduce-Only / Fixed-Point — match
- §UUIDv7 Order ID — match
- §Pending / Rate Limiting / Circuit Breaker — match
- §Heartbeat — drift (spec marks TODO; tests exist in `heartbeat_test.rs`)
- §Pre-validation — match
- §E2E Tests Order Lifecycle — match
- §E2E quic_new_order / quic_cancel — drift (QUIC not implemented)
- §E2E Multi-User / Market Data / Failure / Liquidation — unshipped
- §Implementation Status table (2026-02-10) — drift (heartbeat DONE, stale date)
- §Benchmarks — match
- §Integration Points — match

**Actions:**
- Update §Enum Validation to FailureReason 0-12
- Remove QUIC test names (not implemented)
- Update §Implementation Status (heartbeat DONE; remove stale date)
- Strip test-name lists from §Unit Tests
- Mark unshipped E2E sections explicitly
