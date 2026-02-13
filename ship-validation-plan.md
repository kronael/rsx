# RSX Ship Validation Plan - Full Integration & Stress Testing

## Context

**Current State:**
- All components compile and start (Gateway, Risk, Matching, Marketdata, Mark)
- 1,158 unit/API tests passing (960 Rust + 50 API + 148 Playwright)
- Zero clippy warnings, zero test failures
- Never tested: full system integration, 10k orders/sec stress, replica failover

**Goal:** Validate full system works at production scale before shipping

**Target Metrics (User Specified):**
- 10,000 orders/sec sustained throughput
- 3 symbols concurrent (BTC, ETH, SOL)
- 1,000 users
- <50us Gateway→ME→Gateway latency
- <500ns ME match time

## Phase 1: Build Real Stress Test Infrastructure (1 day)

### 1.1 WebSocket Order Feeder (`rsx-stress/`)

**Create new crate:** `rsx-stress` - WebSocket client stress tester

**Features:**
- Concurrent WebSocket connections (configurable, default 100)
- Order generation: random prices around market, 50/50 buy/sell
- Rate limiting: configurable orders/sec per connection
- User distribution: 1,000 virtual users (round-robin)
- Symbol distribution: BTC 50%, ETH 30%, SOL 20%
- Metrics collection: submission rate, latency (submit→ack), errors
- Output: CSV with timestamp, oid, latency_us, status

**Implementation:**
```rust
// rsx-stress/src/main.rs
// - tokio-tungstenite WebSocket client pool
// - Config: target_rate, num_connections, duration_sec, symbols
// - Main loop: spawn N connections, each submits orders at rate/N
// - Collect: latency histogram (p50/p95/p99), error rate
// - Output: metrics.csv + summary stats
```

**Files:**
- `rsx-stress/Cargo.toml` - tokio, tokio-tungstenite, serde, csv
- `rsx-stress/src/main.rs` - CLI with clap
- `rsx-stress/src/client.rs` - WebSocket client
- `rsx-stress/src/generator.rs` - Order generation logic
- `rsx-stress/src/metrics.rs` - Latency tracking

### 1.2 Full System Orchestrator (`scripts/run-full-system.sh`)

**Bash script to start all processes:**
```bash
#!/bin/bash
# Start: postgres, risk (shard 0), 3 MEs (BTC/ETH/SOL), gateway, marketdata, mark
# Each process: background, PID to file, log to ./log/{process}.log
# Wait: health checks on all components before declaring ready
# Stop: kill all PIDs, cleanup
```

**Process startup order:**
1. Postgres (testcontainer or existing)
2. Risk shard 0 (listen UDP 9090)
3. Matching engines: BTC (9100), ETH (9101), SOL (9102)
4. Marketdata (listen WS 8081)
5. Mark price aggregator (mock feeds)
6. Gateway (listen WS 8080)

**Health checks:**
- Risk: UDP bind successful (check log)
- MEs: Config loaded, book initialized
- Gateway: HTTP GET /health returns 200
- Marketdata: WS connection accepted

### 1.3 Metrics Collection (`scripts/collect-metrics.sh`)

**Scrape structured logs for metrics:**
- Parse `[INFO]` lines with JSON payloads
- Extract: order_latency_us, fill_latency_us, queue_depth, WAL_lag_ms
- Aggregate: p50/p95/p99, throughput (orders/sec, fills/sec)
- Output: metrics.json + prometheus-style text

## Phase 2: Baseline Integration Test (Half day)

### 2.1 Single-Symbol Smoke Test

**Test:** 100 orders/sec for 60 seconds to BTC only

**Setup:**
```bash
./scripts/run-full-system.sh
sleep 10  # warm-up
./target/release/rsx-stress \
  --gateway ws://localhost:8080 \
  --rate 100 \
  --duration 60 \
  --symbols BTCUSD \
  --users 10 \
  --output smoke-test.csv
./scripts/collect-metrics.sh
```

**Success Criteria:**
- Gateway accepts all connections (0 connection errors)
- 6,000 orders submitted (100/sec × 60s)
- >95% orders accepted (not rejected by risk)
- >50% orders filled (matching works)
- Latency p99 <1ms (relaxed, just prove it works)
- No process crashes
- WAL files created and rotated

**Failure modes to check:**
- Connection refused → Gateway not listening
- Orders rejected → Risk margin checks failing
- No fills → ME not running or not connected
- High latency → Backpressure or contention

### 2.2 Multi-Symbol Integration Test

**Test:** 300 orders/sec split across BTC/ETH/SOL for 60 seconds

**Setup:**
```bash
./target/release/rsx-stress \
  --rate 300 \
  --symbols BTCUSD,ETHUSD,SOLUSD \
  --users 100 \
  --duration 60 \
  --output multi-symbol.csv
```

**Success Criteria:**
- 18,000 orders submitted (300/sec × 60s)
- Orders distributed across 3 symbols
- All 3 MEs processing orders (check logs)
- No symbol starvation (each ME gets traffic)
- Latency p99 <2ms

## Phase 3: Real Stress Test - 10k Orders/Sec (1 day)

### 3.1 Ramp-Up Test

**Test:** Gradually increase load to find breaking point

**Stages:**
1. 1,000 orders/sec × 60s
2. 2,500 orders/sec × 60s
3. 5,000 orders/sec × 60s
4. 7,500 orders/sec × 60s
5. 10,000 orders/sec × 60s
6. 12,500 orders/sec × 60s (stretch goal)

**For each stage:**
- Record: throughput achieved, latency p50/p95/p99, error rate
- Check: CPU usage (Risk, MEs, Gateway), memory, WAL lag
- Monitor: queue depths, backpressure triggers

**Failure criteria:**
- Error rate >5% → System overloaded
- Latency p99 >10ms → Unacceptable for HFT
- Process crash → Bug or resource exhaustion
- WAL lag >100ms → Replication falling behind

### 3.2 Sustained 10k Orders/Sec

**Test:** Run at target load for 10 minutes

**Setup:**
```bash
./target/release/rsx-stress \
  --rate 10000 \
  --symbols BTCUSD,ETHUSD,SOLUSD \
  --users 1000 \
  --duration 600 \
  --connections 200 \
  --output stress-10k.csv
```

**Success Criteria:**
- 600,000 orders submitted (10k/sec × 600s)
- Throughput >9,500 orders/sec sustained (95% of target)
- Latency p50 <100us, p95 <500us, p99 <1ms
- Error rate <1%
- All processes stable (no crashes, no OOM)
- WAL lag <50ms throughout
- CPU usage <80% (headroom for bursts)

**Collect Metrics:**
- orders_submitted, orders_accepted, orders_rejected
- fills_executed, avg_fill_time_us
- gateway_latency_us (submit→ack)
- me_latency_ns (insert→match)
- risk_latency_us (margin check)
- wal_write_latency_us, wal_lag_ms
- cpu_percent, mem_mb (per process)

### 3.3 Burst Test

**Test:** Spike to 20k orders/sec for 30 seconds

**Purpose:** Verify backpressure handling, queue resilience

**Success Criteria:**
- System survives (no crashes)
- Graceful degradation (latency increases but bounded)
- Recovery after burst (latency returns to baseline)
- No data loss (all orders logged to WAL)

## Phase 4: Replica Failover Test (Half day)

### 4.1 Setup Primary + Replica

**Start 2 Risk shards:**
- Primary: `RSX_RISK_SHARD_ID=0 RSX_RISK_REPLICA=false`
- Replica: `RSX_RISK_SHARD_ID=0 RSX_RISK_REPLICA=true`

Both connect to same Postgres, both consume DXS from MEs.

**Verify:**
- Primary acquires advisory lock (check logs)
- Replica in standby mode (not processing orders)
- Replica tip tracking primary (check WAL seq)

### 4.2 Failover Test

**Test:** Kill primary during load, replica promotes

**Procedure:**
1. Start stress test at 1,000 orders/sec
2. After 30s, kill primary: `kill -9 {primary_pid}`
3. Replica detects lease loss, acquires lock, promotes
4. Stress test continues (may see temporary errors)
5. Verify replica now processing orders

**Success Criteria:**
- Replica promotes within 2 seconds
- Order processing resumes (stress test recovers)
- No data loss (all orders in WAL)
- Downtime <5 seconds (acceptable for HA)

**Measure:**
- Time to detect primary failure
- Time to acquire lease and promote
- Number of orders lost during failover (should be 0)

### 4.3 Switchback Test

**Test:** Primary rejoins as replica

**Procedure:**
1. Restart killed primary with `RSX_RISK_REPLICA=true`
2. Verify it joins as replica (doesn't contest lock)
3. Verify it syncs from WAL (tip matches promoted replica)
4. Kill promoted replica (now primary)
5. Original primary promotes

**Success Criteria:**
- Primary rejoins without disrupting traffic
- Double-promotion prevented (only one active)
- Syncs WAL correctly (no missing orders)

## Phase 5: Validation & Profiling (Half day)

### 5.1 Latency Breakdown

**Instrument critical path:**
- Gateway: WS receive → CMP send
- Risk: CMP receive → margin check → CMP forward
- ME: CMP receive → match → CMP send fills
- Gateway: CMP receive fills → WS send to user

**Measure:**
- Gateway→Risk: UDP latency
- Risk margin check: computation time
- Risk→ME: UDP latency
- ME match: book operation time
- ME→Marketdata: fanout time
- Fill propagation: ME→Risk→Gateway→User

**Target:**
- Total <50us (Gateway→ME→Gateway)
- ME match <500ns

### 5.2 Throughput Ceiling

**Find maximum sustained throughput:**
- Gradually increase rate beyond 10k/sec
- Monitor: error rate, latency p99, CPU saturation
- Identify bottleneck: Gateway, Risk, ME, or network

**Document:**
- Max achieved: X orders/sec
- Bottleneck: {component} at {resource}
- Headroom: X% below saturation
- Scaling path: how to increase capacity

### 5.3 Resource Usage Under Load

**Profile at 10k orders/sec:**
- CPU: per-process breakdown (htop)
- Memory: RSS, heap allocations (perf)
- Network: UDP packet rate, bandwidth
- Disk I/O: WAL write rate, fsync latency

**Identify:**
- Hot functions (perf record, flamegraph)
- Lock contention (if any)
- Allocation hot spots (jemalloc stats)

## Phase 6: Final Validation Checklist

### 6.1 Spec Compliance

Run full spec verification:
```bash
cargo test --all  # All 960 unit tests
make e2e          # Component integration tests
make integration  # Testcontainers Postgres tests
./scripts/verify-invariants.sh  # Check CONSISTENCY.md
```

**Verify:**
- All specs implemented (PROGRESS.md at 100%)
- All consistency guarantees hold
- Wire protocol matches WEBPROTO.md
- WAL format matches DXS.md

### 6.2 Production Readiness

**Checklist:**
- [ ] 10k orders/sec sustained for 10min
- [ ] Latency p99 <1ms at load
- [ ] Replica failover <5s downtime
- [ ] Zero data loss during failover
- [ ] No crashes under stress
- [ ] WAL lag <50ms sustained
- [ ] CPU headroom >20%
- [ ] Memory stable (no leaks)
- [ ] All 960 tests passing
- [ ] Playground dashboard functional
- [ ] Monitoring/alerting configured

## Deliverables

### Code
1. `rsx-stress/` - WebSocket stress test client
2. `scripts/run-full-system.sh` - Full stack orchestrator
3. `scripts/collect-metrics.sh` - Metrics aggregation
4. `scripts/verify-invariants.sh` - Consistency checks

### Documentation
1. `STRESS-TEST-RESULTS.md` - All test runs with metrics
2. `PERFORMANCE.md` - Latency breakdown, throughput limits
3. `FAILOVER.md` - Replica promotion procedure and timing
4. `SHIP-VALIDATED.md` - Final ship decision with evidence

### Metrics
1. `stress-10k.csv` - 10min stress test raw data
2. `metrics.json` - Aggregated performance metrics
3. `flamegraph.svg` - CPU profile under load
4. `latency-histogram.png` - P50/P95/P99 over time

## Timeline

- **Day 1:** Phase 1 (stress infrastructure) + Phase 2 (smoke tests)
- **Day 2:** Phase 3 (10k stress) + Phase 4 (failover)
- **Day 3:** Phase 5 (profiling) + Phase 6 (final validation)

**Total: 3 days**

## Success Criteria (Go/No-Go)

**GO if:**
- ✅ 10k orders/sec sustained with <1% error rate
- ✅ Latency p99 <1ms under load
- ✅ Replica failover works (<5s downtime)
- ✅ Zero crashes, zero data loss
- ✅ All 960 tests passing

**NO-GO if:**
- ❌ Cannot sustain >8k orders/sec
- ❌ Latency p99 >5ms
- ❌ Failover >30s or loses orders
- ❌ Crashes under stress
- ❌ Test failures or data corruption

## Risk Mitigation

**If stress test fails:**
1. Profile bottleneck (Gateway, Risk, or ME)
2. Optimize hot path (reduce allocations, lock-free)
3. Re-test with fix
4. If still failing, document limits and ship with lower target

**If failover fails:**
1. Debug lease acquisition logic
2. Add more logging/tracing
3. Test lease timeout tuning
4. Fallback: ship without HA, document as v1.1 feature

**If validation takes >3 days:**
- Ship with partial validation (smoke tests only)
- Document known gaps
- Plan v1.1 with full stress testing
