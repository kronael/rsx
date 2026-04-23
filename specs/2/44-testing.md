---
status: shipped
---

# Testing Strategy

For comprehensive edge case documentation across all validation layers,
see [VALIDATION-EDGE-CASES.md](VALIDATION-EDGE-CASES.md).

## Table of Contents

- [Test Organization](#test-organization)
- [1. `make test` - Unit Tests (<5s)](#1-make-test---unit-tests-5s)
- [2. `make e2e` - E2E Tests (~30s)](#2-make-e2e---e2e-tests-30s)
- [3. `make integration` - Integration Tests](#3-make-integration---integration-tests-with-testcontainers)
- [4. `make wal` - WAL Correctness Tests (<10s)](#4-make-wal---wal-correctness-tests-10s)
- [5. `make smoke` - Smoke Tests](#5-make-smoke---smoke-tests-against-deployed-systems)
- [6. `make perf` - Performance Benchmarks](#6-make-perf---performance-benchmarks-long-running)
- [Test Data Patterns](#test-data-patterns)
- [Correctness Invariants](#correctness-invariants)
- [Test Framework & Tooling](#test-framework--tooling)
- [CI/CD Pipeline](#cicd-pipeline)
- [Deferred to Future](#deferred-to-future)
- [Component Test Specs](#component-test-specs)
- [References](#references)

---

## Test Organization

RSX testing is organized into five levels with corresponding make targets,
providing a clear workflow from fast iteration to full validation to
performance characterization.

---

## 1. `make test` - Unit Tests (<5s)

**Purpose:** Fast feedback loop for development. Run on every code change.

**Actual Status:** 895 tests passing across all crates. Zero failures,
all non-flaky (unique temp dirs, proper cleanup, ephemeral ports).

**Scope:**
- Slab allocator operations (alloc, free, free list)
- Compressed zone lookup (bisection, boundary conditions)
- Price/qty validation (tick size, lot size alignment)
- Order insertion/matching/cancellation in isolation
- Best bid/ask tracking after order removal
- Event generation correctness
- CompressionMap index calculations

**Characteristics:**
- No external dependencies
- Pure functions, mocked where needed
- Runs in <5s total
- CI: every commit

---

## 2. `make e2e` - E2E Tests (~30s)

**Purpose:** Validate complete order lifecycle with mocked components where
appropriate. Run on every PR.

**Scope:**
- Single user, single symbol scenarios
- Full order lifecycle: submit → match → fills → completion
- Mocked transports for isolation (real orderbook + mocked CMP where needed)
- Edge cases:
  - Pre-trade margin check failure
  - Duplicate order_id (5min dedup window)
  - Cancel before/after partial fill
  - Multi-fill sequences (large taker crosses multiple makers)
  - Out-of-order response handling (LIFO VecDeque)
- Event fan-out verification (risk/gateway/mktdata get correct events)
- ORDER_DONE precedes all fills (ref MESSAGES.md)

**Characteristics:**
- Mocked data and transport where appropriate
- Real orderbook logic
- Tests complete request/response flows
- Runs in ~30s total
- CI: every PR

---

## 3. `make integration` - Integration Tests (with testcontainers)

**Purpose:** Full system stack validation with real components. Run on every
PR or on-demand.

**Scope:**
- Full system stack running (gateway, matching, risk, mktdata)
- Real CMP/UDP links (not mocked)
- Multi-symbol, multi-user scenarios
- Tail event handling:
  - 50% crash/rally triggers recentering
  - Rapid orders during migration
  - Incremental recentering verification (frontiers expand correctly)
  - Smooshed tick matching (check exact prices in coarse zones)
- High-frequency patterns:
  - Rapid inserts/cancels/matches
  - Memory leak detection (slab reuse verification)
  - Slab free list correctness
- Failure mode testing:
  - Matching engine crash/restart (book starts empty)
  - Risk engine crash/restart (positions persisted)
  - CMP flow control/backpressure verification
  - Network partition (circuit breaker behavior)

**Characteristics:**
- Testcontainers for services
- Real CMP/UDP links
- Multi-process scenarios
- Runs in variable time (typically 1-5min)
- CI: every PR or on-demand

---

## 4. `make wal` - WAL Correctness Tests (<10s)

**Purpose:** Validate fixed-record WAL format and replay.

**Scope:**
- Record header parsing (version/type/len)
- Fixed-record encode/decode (little-endian)
- Sequence monotonicity
- Replay from tip + 1

---

## 5. `make smoke` - Smoke Tests (against deployed systems)

**Purpose:** Quick validation that deployed systems are operational. Run
post-deployment.

**Scope:**
- Run against live/staging deployments
- Basic health checks:
  - Order submission works (simple buy/sell)
  - Cancellation works
  - Position updates propagate to risk engine
  - Market data streaming works
- No destructive operations
- Quick validation (<1min)
- Non-exhaustive (just "is it alive?")

**Characteristics:**
- Against real deployments
- Read-mostly, minimal writes
- Fast (<1min)
- CI: post-deployment hook

**Example tests:**
```rust
#[test]
fn can_submit_and_cancel_order() { ... }

#[test]
fn market_data_stream_active() { ... }

#[test]
fn risk_engine_responds() { ... }
```

---

## 6. `make perf` - Performance Benchmarks (long-running)

**Purpose:** Performance characterization and regression detection. Run
nightly or on-demand.

### Single-Threaded Orderbook Benchmarks

**Targets:**
- Insert: 100-500ns (p50/p99/p99.9)
- Match: 100-500ns
- Cancel: 100-300ns
- Recentering: lazy migration ~1-3us, normal <1us

**Methodology:**
- Criterion for statistical analysis
- Warmup cycles (avoid cold caches)
- Multiple runs (detect variance)
- Regression detection (fail if >10% slower)

### Memory Efficiency

**Targets:**
- 78M orders in ~10GB (128B slots)
- Price level arrays: ~30MB (two arrays, ~617K slots each)
- Event buffer: ~1.3MB (10K events)
- Total per book: ~10GB

**Verification:**
- Measure actual RSS/heap usage
- Verify slab reuse (no memory leaks)
- Confirm pre-allocation (no malloc on hot path)

### Load Tests

**Normal load: 10K orders/sec sustained 10min**
- 1000 users, 100 symbols
- Zipf distribution: BTC-PERP 30%, long tail alts
- Verify: latency stable, no memory growth

**Burst load: 100K orders/sec spike 10s**
- Sudden traffic spike (simulated news event)
- Verify: system handles burst, recovers gracefully

**Tail event cascade: BTC-PERP drops 50%**
- Mid-price drops from $50K → $25K
- Orders across all 5 zones
- Verify: recentering completes, matching continues

**Liquidation cascade: 100 users liquidated**
- Simulate margin calls (100 users breach maintenance margin)
- Verify: liquidation orders execute, positions closed

**Symbol hotspot: 90% traffic on BTC-PERP**
- Unbalanced load across symbols
- Verify: BTC-PERP orderbook handles load, other symbols unaffected

### E2E Latency Measurement

**Target: <50us end-to-end (same machine, CMP/UDP)**

**Breakdown:**
- Gateway: receive order → send to matching (~5-10us)
- CMP/UDP: send → recv (kernel bound)
- Matching: insert + match + event gen (~100-500ns)
- CMP/UDP: event send → recv (kernel bound)
- Risk: update position (~1-5us)
- Total: <50us (same machine, dedicated cores)

**Methodology:**
- Timestamp at each stage
- TSC (rdtsc) for nanosecond precision
- Flame graphs for bottleneck identification
- Statistical distribution (p50/p99/p99.9)

### WAL Benchmark (Fixed-Record Baseline)

**Purpose:** Compare fixed-record WAL append vs baseline serialization.

**Method:**
- Fixed-record: write header+payload (raw #[repr(C)] records).
- Baseline: serialize equivalent event with length-prefix.
- Report records/sec, MB/sec, CPU%.

### Profiling & Metrics

**Tools:**
- `perf record/report` - CPU profiling
- `perf stat` - cache miss rates, branch predictions
- Flame graphs - visual call stack analysis
- cachegrind - cache simulation
- tokio-console - async task tracing

**Metrics (Prometheus + Grafana):**
- Orders/sec (throughput)
- Match latency histogram
- CMP flow control counters (backpressure indicator)
- Slab utilization (allocated vs free)
- Recentering frequency

**CI: nightly or on-demand**

---

## Test Data Patterns

### Normal Market Conditions
- 1-2 tick spread
- Most orders in zone 0 (±5% of mid)
- Balanced buy/sell pressure
- Typical: 60% immediate match, 40% resting

### Tail Event (50% crash)
- Mid-price drops rapidly
- Orders across all 5 zones
- Heavy selling pressure
- Zone 4 catch-all populated
- Recentering triggered

### Flash Crash
- Mid drops 50% in <1s
- Orderbook nearly empty on bid side
- Wide spread (10%+)
- Recovery: mid rebounds, orders repopulate

### Whale Order
- Single taker crosses 500 makers
- Tests multi-fill sequences
- Tests event buffer capacity (10K events)
- Verifies O(k) smooshed tick matching

### Zipf Distribution (Symbol Traffic)
- BTC-PERP: 30% of orders
- Top 10 symbols: 70% of orders
- Long tail: 100+ symbols, <1% each
- Tests multi-symbol fairness

---

## Correctness Invariants

Verified across all test levels:

1. **Fills precede ORDER_DONE**
   - Every FILL message comes before ORDER_DONE
   - ORDER_DONE signals "no more fills"

2. **Exactly-one completion per order**
   - Every NewOrder gets exactly one: ORDER_DONE OR ORDER_FAILED
   - Never both, never zero (unless network failure)

3. **FIFO within price level**
   - Orders at same price execute in time priority
   - Earlier timestamp = earlier match

4. **Position consistency**
   - Sum of fills = position delta
   - Risk engine position matches orderbook fills

5. **No negative qty in orderbook**
   - Order qty ≥ 0 at all times
   - After full fill, order removed (not qty=0 resting)

6. **Best bid/ask coherence**
   - best_bid < best_ask (no crossed book)
   - best_bid/ask point to populated levels

7. **Event ordering preserved per CMP stream**
   - Events arrive at consumers in FIFO order
   - seq monotonic within symbol

8. **Slab no-leak**
   - allocated = free + active
   - Free list correctness (no cycles, no lost slots)

---

## Test Framework & Tooling

### Unit Tests
- `#[test]` + `cargo test`
- Fast, isolated, no external deps
- Run with `make test`

### E2E Tests
- Mock CMP for isolation
- Real CMP for full stack
- Custom test harness (order submission helpers)
- Run with `make e2e`

### Integration Tests
- `tests/` directory (separate from `src/`)
- testcontainers-rs for services
- Real CMP/UDP links
- Run with `make integration`

### Benchmarks
- Criterion (statistical regression detection)
- Custom latency measurement (TSC/rdtsc)
- Flame graphs (`perf record` + `flamegraph`)
- Run with `make perf`

### Property Testing
- quickcheck/proptest (future)
- Generate random order sequences
- Verify invariants hold under all inputs

### Tracing & Debugging
- tokio-console (async task inspection)
- tracing crate (structured logging)
- Flame graphs (performance bottlenecks)
- perf/cachegrind (cache analysis)

### Load Generation
- Custom 1000-user simulator
- Zipf distribution for symbol selection
- Configurable order rate (orders/sec)
- Tail event injection (crash/rally scenarios)

---

## CI/CD Pipeline

| Make Target | Trigger | Duration | Failure Action |
|-------------|---------|----------|----------------|
| `make test` | Every commit | <5s | Block merge |
| `make e2e` | Every PR | ~30s | Block merge |
| `make integration` | Every PR or on-demand | 1-5min | Block merge |
| `make smoke` | Post-deployment | <1min | Rollback |
| `make perf` | Nightly | 10-60min | Alert (non-blocking) |

**Branch protection:**
- `make test` + `make e2e` required for merge
- `make integration` optional (on-demand for risky changes)

**Performance tracking:**
- `make perf` results stored (trend analysis)
- Regression alerts (>10% slower = warning)
- Grafana dashboards for historical trends

---

## Deferred to Future

### Cross-Symbol Portfolio Margining
- Multi-symbol position limits
- Cross-margining calculations
- Correlation-based risk

### Orderbook Checkpointing
- Snapshot creation
- Snapshot restoration
- Incremental checkpoints

---

## Component Test Specs

Each component has a dedicated test spec with requirements
checklist, unit tests, e2e tests, benchmarks, and integration
points.

| Component | Test Spec | Source Spec |
|-----------|-----------|-------------|
| SPSC ring buffer | [TESTING-SMRB.md](TESTING-SMRB.md) | notes/SMRB.md |
| Shared orderbook | [TESTING-BOOK.md](TESTING-BOOK.md) | ORDERBOOK.md |
| DXS (WAL + replay) | [TESTING-DXS.md](TESTING-DXS.md) | DXS.md, WAL.md |
| Matching engine | [TESTING-MATCHING.md](TESTING-MATCHING.md) | ORDERBOOK.md, CONSISTENCY.md |
| Risk engine | [TESTING-RISK.md](TESTING-RISK.md) | RISK.md |
| Liquidator | [TESTING-LIQUIDATOR.md](TESTING-LIQUIDATOR.md) | LIQUIDATOR.md |
| Mark price | [TESTING-MARK.md](TESTING-MARK.md) | MARK.md |
| Gateway | [TESTING-GATEWAY.md](TESTING-GATEWAY.md) | NETWORK.md, WEBPROTO.md, RPC.md, MESSAGES.md |
| Market data | [TESTING-MARKETDATA.md](TESTING-MARKETDATA.md) | MARKETDATA.md |

---

## References

- [ORDERBOOK.md](ORDERBOOK.md) - Matching internals, data structures
- [MESSAGES.md](MESSAGES.md) - CMP/WAL wire format, message definitions
- [CONSISTENCY.md](CONSISTENCY.md) - Event fan-out, ordering guarantees
- [SMRB.md](../../notes/SMRB.md) - SPSC ring buffer design
- [NETWORK.md](NETWORK.md) - System topology, component communication
