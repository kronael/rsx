# RSX Monitoring & Alerts

Metrics, dashboards, and alert thresholds for the RSX perpetuals exchange.
Monitoring validates the guarantees in [GUARANTEES.md](GUARANTEES.md) and
enables proactive detection of failures before user impact.

**Philosophy:**
- Metrics drive alerts (data-driven, not gut-feel)
- Alerts must be actionable (every alert has runbook reference)
- P0 alerts = data loss risk (page immediately)
- P1 alerts = degraded service (page during hours)
- P2 alerts = shadow component down (email/Slack)

---

## 1. Metrics by Component

### 1.1 Matching Engine (ME)

| Metric | Type | Description | Target | Unit |
|--------|------|-------------|--------|------|
| `me_heartbeat_last_seen_sec` | Gauge | Seconds since last heartbeat | <5 | seconds |
| `me_orders_accepted_total` | Counter | Orders accepted by ME | - | count |
| `me_orders_matched_total` | Counter | Orders matched (filled) | - | count |
| `me_fills_emitted_total` | Counter | Fills emitted to consumers | - | count |
| `me_wal_append_latency_us` | Histogram | WAL append latency | p99 <1 | us |
| `me_wal_flush_latency_ms` | Histogram | WAL flush latency (fsync) | p99 <10 | ms |
| `me_wal_flush_lag_ms` | Histogram | Time since last flush | p99 <10 | ms |
| `me_wal_buffer_bytes` | Gauge | Bytes in WAL buffer | <64MB | bytes |
| `me_wal_seq` | Gauge | Current WAL seq | - | seq |
| `me_wal_seq_gap_total` | Counter | Seq gaps detected | 0 | count |
| `me_backpressure_stalls_total` | Counter | Stalls due to backpressure | 0 | count |
| `me_spsc_ring_full_total` | Counter | SPSC ring full events | 0 | count |
| `me_orderbook_orders` | Gauge | Active orders in book | - | count |
| `me_orderbook_levels` | Gauge | Active price levels | - | count |
| `me_slab_allocated` | Gauge | Slab slots allocated | - | count |
| `me_slab_free` | Gauge | Slab slots free | >1000 | count |
| `me_snapshot_duration_ms` | Histogram | Snapshot save duration | p99 <100 | ms |
| `me_replay_duration_ms` | Histogram | WAL replay duration | p99 <5000 | ms |
| `me_dxs_consumers` | Gauge | Active DXS consumers | - | count |

### 1.2 Risk Engine

| Metric | Type | Description | Target | Unit |
|--------|------|-------------|--------|------|
| `risk_heartbeat_last_seen_sec` | Gauge | Seconds since last heartbeat | <5 | seconds |
| `risk_fills_received_total` | Counter | Fills received from ME | - | count |
| `risk_fills_applied_total` | Counter | Fills applied to positions | - | count |
| `risk_fills_duplicate_total` | Counter | Duplicate fills (deduped) | - | count |
| `risk_orders_accepted_total` | Counter | Orders accepted from Gateway | - | count |
| `risk_orders_rejected_total` | Counter | Orders rejected (margin) | - | count |
| `risk_positions_count` | Gauge | Active positions | - | count |
| `risk_users_count` | Gauge | Users with positions | - | count |
| `risk_margin_recalc_duration_us` | Histogram | Margin recalc per user | p99 <10 | us |
| `risk_liquidations_triggered_total` | Counter | Liquidations triggered | - | count |
| `risk_funding_settlements_total` | Counter | Funding settlements | - | count |
| `risk_postgres_write_latency_ms` | Histogram | Postgres write latency | p99 <10 | ms |
| `risk_postgres_write_lag_ms` | Histogram | Time since last flush | p99 <10 | ms |
| `risk_postgres_buffer_bytes` | Gauge | Bytes in write buffer | <10MB | bytes |
| `risk_postgres_conn_errors_total` | Counter | Postgres connection errors | 0 | count |
| `risk_replica_lag_ms` | Histogram | Replica lag behind main | p99 <100 | ms |
| `risk_tips_per_symbol` | Gauge | Last seq per symbol | - | seq |
| `risk_advisory_lock_held` | Gauge | Advisory lock status (0 or 1) | 1 | bool |
| `risk_dxs_replay_duration_ms` | Histogram | DXS replay duration | p99 <5000 | ms |

### 1.3 Gateway

| Metric | Type | Description | Target | Unit |
|--------|------|-------------|--------|------|
| `gateway_heartbeat_last_seen_sec` | Gauge | Seconds since last heartbeat | <5 | seconds |
| `gateway_connections_active` | Gauge | Active WebSocket connections | - | count |
| `gateway_orders_received_total` | Counter | Orders received from users | - | count |
| `gateway_orders_routed_total` | Counter | Orders routed to Risk | - | count |
| `gateway_orders_rejected_total` | Counter | Orders rejected (validation) | - | count |
| `gateway_fills_broadcast_total` | Counter | Fills broadcast to users | - | count |
| `gateway_order_latency_ms` | Histogram | Order submission latency | p99 <5 | ms |
| `gateway_risk_conn_errors_total` | Counter | Risk connection errors | 0 | count |

### 1.4 Market Data (MARKETDATA)

| Metric | Type | Description | Target | Unit |
|--------|------|-------------|--------|------|
| `marketdata_heartbeat_last_seen_sec` | Gauge | Seconds since last heartbeat | <5 | seconds |
| `marketdata_events_received_total` | Counter | Events received from ME | - | count |
| `marketdata_bbo_updates_total` | Counter | BBO updates emitted | - | count |
| `marketdata_l2_updates_total` | Counter | L2 updates emitted | - | count |
| `marketdata_shadow_book_orders` | Gauge | Orders in shadow book | - | count |
| `marketdata_shadow_book_levels` | Gauge | Levels in shadow book | - | count |
| `marketdata_me_lag_ms` | Histogram | Lag behind ME seq | p99 <100 | ms |

### 1.5 Postgres

| Metric | Type | Description | Target | Unit |
|--------|------|-------------|--------|------|
| `postgres_up` | Gauge | Postgres availability | 1 | bool |
| `postgres_connections_active` | Gauge | Active connections | - | count |
| `postgres_transactions_total` | Counter | Transactions committed | - | count |
| `postgres_xact_commit_duration_ms` | Histogram | Transaction commit time | p99 <10 | ms |
| `postgres_locks_waiting` | Gauge | Waiting locks | 0 | count |
| `postgres_autovacuum_running` | Gauge | Autovacuum active | - | bool |
| `postgres_disk_usage_pct` | Gauge | Disk usage percentage | <80 | percent |

---

## 2. Alert Thresholds

### 2.1 P0: Data Loss Risk (Page Immediately)

| Alert | Condition | Threshold | Runbook |
|-------|-----------|-----------|---------|
| ME seq gap | `me_wal_seq_gap_total > 0` | 0 gaps | RECOVERY-RUNBOOK.md §5.1 |
| Position mismatch | Reconciliation delta > 0 | 0 delta | RECOVERY-RUNBOOK.md §5.2 |
| Advisory lock lost | `risk_advisory_lock_held == 0` | Must be 1 | RECOVERY-RUNBOOK.md §4.1 |
| Split-brain | `COUNT(advisory_lock) > 1` | Must be 1 | RECOVERY-RUNBOOK.md §4.2 |
| Funding not zero-sum | `ABS(SUM(funding)) > 1bps` | Must be 0 | RECOVERY-RUNBOOK.md §5.2 |
| Tips decreased | `tips[symbol_id] < prev_tip` | Never decrease | RECOVERY-RUNBOOK.md §5.2 |

### 2.2 P1: Degraded Service (Page During Hours)

| Alert | Condition | Threshold | Runbook |
|-------|-----------|-----------|---------|
| ME heartbeat timeout | `me_heartbeat_last_seen_sec > 5` | <5s | RECOVERY-RUNBOOK.md §2.1 |
| Risk heartbeat timeout | `risk_heartbeat_last_seen_sec > 5` | <5s | RECOVERY-RUNBOOK.md §2.2 |
| Gateway heartbeat timeout | `gateway_heartbeat_last_seen_sec > 5` | <5s | RECOVERY-RUNBOOK.md §2.3 |
| Postgres down | `postgres_up == 0` | Must be 1 | RECOVERY-RUNBOOK.md §2.4 |
| ME WAL flush lag | `me_wal_flush_lag_ms p99 > 15` | <10ms (warn), <50ms (crit) | RECOVERY-RUNBOOK.md §3.1 |
| Risk Postgres lag | `risk_postgres_write_lag_ms p99 > 50` | <10ms (warn), <100ms (crit) | RECOVERY-RUNBOOK.md §3.2 |
| Risk replica lag | `risk_replica_lag_ms p99 > 500` | <100ms (warn), <1000ms (crit) | RECOVERY-RUNBOOK.md §3.3 |
| ME backpressure stalls | `rate(me_backpressure_stalls_total) > 10` | 0/sec | RECOVERY-RUNBOOK.md §3.1 |
| Postgres locks waiting | `postgres_locks_waiting > 5` | 0 | RECOVERY-RUNBOOK.md §3.2 |

### 2.3 P2: Shadow Component Down (Email/Slack)

| Alert | Condition | Threshold | Runbook |
|-------|-----------|-----------|---------|
| ME replica offline | `me_replica_heartbeat > 10` | <5s | RECOVERY-RUNBOOK.md §2.1 |
| Risk replica offline | `risk_replica_heartbeat > 10` | <5s | RECOVERY-RUNBOOK.md §2.2 |
| MARKETDATA offline | `marketdata_heartbeat > 10` | <5s | RECOVERY-RUNBOOK.md §2 |
| Postgres replica offline | `postgres_replica_up == 0` | Must be 1 | RECOVERY-RUNBOOK.md §2.4 |

---

## 3. Dashboards

### 3.1 System Health (Overview)

**Purpose:** High-level view of all components, for ops team monitoring.

**Panels:**
1. Component heartbeats (all components, color-coded: green/red)
2. Advisory lock status per shard (must be exactly 1)
3. Trading volume (orders/sec, fills/sec, notional/sec)
4. Error rates (rejected orders, connection errors, backpressure stalls)

**Refresh:** 5s

### 3.2 Matching Engine

**Purpose:** Deep dive into ME performance and WAL health.

**Panels:**
1. Orders accepted, matched, fills emitted (rate, 1min window)
2. WAL flush latency (p50/p99/p999, 1min window)
3. WAL flush lag (p99, 1min window)
4. WAL buffer usage (bytes, % of max)
5. Backpressure stalls (rate, 1min window)
6. SPSC ring full events (rate, 1min window)
7. Orderbook state (active orders, levels)
8. Slab allocator (allocated, free, utilization %)
9. DXS consumers (count, per-consumer lag)

**Refresh:** 1s

### 3.3 Risk Engine

**Purpose:** Deep dive into Risk fill processing, margin, and persistence.

**Panels:**
1. Fills received, applied, duplicates (rate, 1min window)
2. Orders accepted, rejected (rate, 1min window)
3. Margin recalc duration (p50/p99, 1min window)
4. Liquidations triggered (rate, 1min window)
5. Postgres write latency (p50/p99, 1min window)
6. Postgres write lag (p99, 1min window)
7. Postgres buffer usage (bytes, % of max)
8. Replica lag (p99, 1min window)
9. Tips per symbol (gauge, latest value)
10. Advisory lock status (gauge, 0 or 1)

**Refresh:** 1s

### 3.4 Gateway

**Purpose:** User-facing metrics, connection health, order latency.

**Panels:**
1. Active connections (gauge)
2. Orders received, routed, rejected (rate, 1min window)
3. Fills broadcast (rate, 1min window)
4. Order latency (p50/p99/p999, 1min window)
5. Risk connection errors (rate, 1min window)

**Refresh:** 1s

### 3.5 Data Integrity

**Purpose:** Verify invariants from GUARANTEES.md.

**Panels:**
1. Position reconciliation status (query every 10min)
   - Sample 1% of users, compare positions vs fills
   - Show: users checked, mismatches found
2. Seq gap detection (continuous)
   - Check ME WAL for gaps
   - Show: symbols checked, gaps found
3. Tips monotonic check (continuous)
   - Check tips table for decreases
   - Show: symbols checked, violations found
4. Funding zero-sum check (after each settlement)
   - Check funding_payments sum per symbol/interval
   - Show: settlements checked, violations found
5. Advisory lock check (continuous)
   - Check pg_locks for advisory lock count per shard
   - Show: shards checked, violations found (!=1)

**Refresh:** 10s (except position reconciliation: 10min)

---

## 4. Metrics Collection

### 4.1 Instrumentation

**Framework:** Prometheus client library (Rust: `prometheus` crate)

**Pattern:**
```rust
// In each component's main.rs
use prometheus::{Registry, Counter, Histogram, Gauge};

lazy_static! {
    pub static ref ME_FILLS_EMITTED: Counter = Counter::new(
        "me_fills_emitted_total",
        "Total fills emitted by ME"
    ).unwrap();

    pub static ref ME_WAL_FLUSH_LAG: Histogram = Histogram::with_opts(
        HistogramOpts::new("me_wal_flush_lag_ms", "WAL flush lag in ms")
            .buckets(vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0])
    ).unwrap();
}

// In hot path
ME_FILLS_EMITTED.inc();
ME_WAL_FLUSH_LAG.observe(lag_ms);

// Expose via HTTP endpoint
#[get("/metrics")]
fn metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
```

**Endpoints:**
- ME: `http://me:9100/metrics`
- Risk: `http://risk:9200/metrics`
- Gateway: `http://gateway:8080/metrics`
- MARKETDATA: `http://marketdata:9300/metrics`

### 4.2 Prometheus Scrape Config

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'rsx-matching'
    scrape_interval: 5s
    static_configs:
      - targets:
        - 'me-btcusd:9100'
        - 'me-ethusd:9100'
        # ... all ME instances
    relabel_configs:
      - source_labels: [__address__]
        regex: 'me-([^:]+):.*'
        target_label: symbol

  - job_name: 'rsx-risk'
    scrape_interval: 5s
    static_configs:
      - targets:
        - 'risk-shard0:9200'
        - 'risk-shard1:9200'
        # ... all Risk shards
    relabel_configs:
      - source_labels: [__address__]
        regex: 'risk-shard([0-9]+):.*'
        target_label: shard_id

  - job_name: 'rsx-gateway'
    scrape_interval: 5s
    static_configs:
      - targets: ['gateway:8080']

  - job_name: 'rsx-marketdata'
    scrape_interval: 5s
    static_configs:
      - targets: ['marketdata:9300']

  - job_name: 'postgres'
    scrape_interval: 10s
    static_configs:
      - targets: ['postgres-exporter:9187']
```

### 4.3 Grafana Dashboards

**Import:** Pre-built dashboards in `/grafana/dashboards/*.json`

**Dashboards:**
1. `system-health.json` - System Health (Overview)
2. `matching-engine.json` - Matching Engine
3. `risk-engine.json` - Risk Engine
4. `gateway.json` - Gateway
5. `data-integrity.json` - Data Integrity

**Variables:**
- `$symbol` - Symbol filter (BTCUSD, ETHUSD, etc.)
- `$shard` - Risk shard ID (0-15)
- `$interval` - Time interval (1m, 5m, 1h)

---

## 5. Alerting

### 5.1 Alertmanager Config

```yaml
# alertmanager.yml
global:
  resolve_timeout: 5m
  slack_api_url: 'https://hooks.slack.com/services/YOUR/WEBHOOK'
  pagerduty_url: 'https://events.pagerduty.com/v2/enqueue'

route:
  group_by: ['alertname', 'severity']
  group_wait: 10s
  group_interval: 10s
  repeat_interval: 1h
  receiver: 'slack-ops'

  routes:
    # P0: Page immediately + Slack
    - match:
        severity: p0
      receiver: 'pagerduty-oncall'
      continue: true
    - match:
        severity: p0
      receiver: 'slack-incidents'

    # P1: Page during hours + Slack
    - match:
        severity: p1
      receiver: 'pagerduty-business-hours'
      continue: true
    - match:
        severity: p1
      receiver: 'slack-ops'

    # P2: Slack only
    - match:
        severity: p2
      receiver: 'slack-ops'

receivers:
  - name: 'pagerduty-oncall'
    pagerduty_configs:
      - service_key: 'YOUR_PD_SERVICE_KEY'
        severity: 'critical'

  - name: 'pagerduty-business-hours'
    pagerduty_configs:
      - service_key: 'YOUR_PD_SERVICE_KEY'
        severity: 'warning'

  - name: 'slack-incidents'
    slack_configs:
      - channel: '#incidents'
        title: 'P0 Alert: {{ .GroupLabels.alertname }}'
        text: '{{ range .Alerts }}{{ .Annotations.summary }}{{ end }}'

  - name: 'slack-ops'
    slack_configs:
      - channel: '#ops'
        title: '{{ .GroupLabels.alertname }}'
        text: '{{ range .Alerts }}{{ .Annotations.summary }}{{ end }}'
```

### 5.2 Prometheus Alert Rules

```yaml
# alerts/guarantees.yml
groups:
  - name: guarantees
    interval: 10s
    rules:
      # P0: Seq gap detected
      - alert: MESeqGap
        expr: me_wal_seq_gap_total > 0
        for: 0s
        labels:
          severity: p0
        annotations:
          summary: "ME seq gap detected for {{ $labels.symbol }}"
          description: "Fill lost (critical bug). See RECOVERY-RUNBOOK.md §5.1"

      # P0: Position mismatch
      - alert: PositionMismatch
        expr: risk_position_reconciliation_mismatch_total > 0
        for: 0s
        labels:
          severity: p0
        annotations:
          summary: "Position mismatch detected for shard {{ $labels.shard_id }}"
          description: "Position drift (critical bug). See RECOVERY-RUNBOOK.md §5.2"

      # P0: Advisory lock lost
      - alert: AdvisoryLockLost
        expr: risk_advisory_lock_held == 0
        for: 10s
        labels:
          severity: p0
        annotations:
          summary: "No main for shard {{ $labels.shard_id }}"
          description: "Both instances down or network partition. See RECOVERY-RUNBOOK.md §4.1"

      # P0: Split-brain
      - alert: SplitBrain
        expr: sum by (shard_id) (risk_advisory_lock_held) > 1
        for: 0s
        labels:
          severity: p0
        annotations:
          summary: "Split-brain detected for shard {{ $labels.shard_id }}"
          description: "Multiple instances hold lock. See RECOVERY-RUNBOOK.md §4.2"

      # P0: Tips decreased
      - alert: TipsDecreased
        expr: (risk_tips_per_symbol - risk_tips_per_symbol offset 1m) < 0
        for: 0s
        labels:
          severity: p0
        annotations:
          summary: "Tips decreased for symbol {{ $labels.symbol_id }}"
          description: "Tips must be monotonic. See RECOVERY-RUNBOOK.md §5.2"

# alerts/performance.yml
groups:
  - name: performance
    interval: 10s
    rules:
      # P1: ME heartbeat timeout
      - alert: MEHeartbeatTimeout
        expr: time() - me_heartbeat_last_seen_sec > 5
        for: 0s
        labels:
          severity: p1
        annotations:
          summary: "ME heartbeat timeout for {{ $labels.symbol }}"
          description: "ME may be down. See RECOVERY-RUNBOOK.md §2.1"

      # P1: Risk heartbeat timeout
      - alert: RiskHeartbeatTimeout
        expr: time() - risk_heartbeat_last_seen_sec > 5
        for: 0s
        labels:
          severity: p1
        annotations:
          summary: "Risk heartbeat timeout for shard {{ $labels.shard_id }}"
          description: "Risk may be down. See RECOVERY-RUNBOOK.md §2.2"

      # P1: WAL flush lag (warning)
      - alert: MEWalFlushLagWarning
        expr: histogram_quantile(0.99, me_wal_flush_lag_ms) > 15
        for: 1m
        labels:
          severity: p1
        annotations:
          summary: "ME WAL flush lag high for {{ $labels.symbol }}"
          description: "p99 > 15ms. Check disk. See RECOVERY-RUNBOOK.md §3.1"

      # P1: WAL flush lag (critical)
      - alert: MEWalFlushLagCritical
        expr: histogram_quantile(0.99, me_wal_flush_lag_ms) > 50
        for: 30s
        labels:
          severity: p1
        annotations:
          summary: "ME WAL flush lag critical for {{ $labels.symbol }}"
          description: "p99 > 50ms. Disk failure? See RECOVERY-RUNBOOK.md §3.1"

      # P1: Postgres write lag (warning)
      - alert: RiskPostgresLagWarning
        expr: histogram_quantile(0.99, risk_postgres_write_lag_ms) > 50
        for: 1m
        labels:
          severity: p1
        annotations:
          summary: "Risk Postgres write lag high for shard {{ $labels.shard_id }}"
          description: "p99 > 50ms. Check DB. See RECOVERY-RUNBOOK.md §3.2"

      # P1: Postgres write lag (critical)
      - alert: RiskPostgresLagCritical
        expr: histogram_quantile(0.99, risk_postgres_write_lag_ms) > 100
        for: 30s
        labels:
          severity: p1
        annotations:
          summary: "Risk Postgres write lag critical for shard {{ $labels.shard_id }}"
          description: "p99 > 100ms. DB failure? See RECOVERY-RUNBOOK.md §3.2"

      # P1: Replica lag
      - alert: RiskReplicaLagHigh
        expr: histogram_quantile(0.99, risk_replica_lag_ms) > 500
        for: 1m
        labels:
          severity: p1
        annotations:
          summary: "Risk replica lag high for shard {{ $labels.shard_id }}"
          description: "p99 > 500ms. Replica slow. See RECOVERY-RUNBOOK.md §3.3"

      # P1: Backpressure stalls
      - alert: MEBackpressureStalls
        expr: rate(me_backpressure_stalls_total[1m]) > 10
        for: 30s
        labels:
          severity: p1
        annotations:
          summary: "ME backpressure stalls for {{ $labels.symbol }}"
          description: "Slow consumer or disk. See RECOVERY-RUNBOOK.md §3.1"

# alerts/shadows.yml
groups:
  - name: shadows
    interval: 10s
    rules:
      # P2: ME replica offline
      - alert: MEReplicaOffline
        expr: time() - me_replica_heartbeat_last_seen_sec > 10
        for: 1m
        labels:
          severity: p2
        annotations:
          summary: "ME replica offline for {{ $labels.symbol }}"
          description: "Replica down. No immediate impact. See RECOVERY-RUNBOOK.md §2.1"

      # P2: Risk replica offline
      - alert: RiskReplicaOffline
        expr: time() - risk_replica_heartbeat_last_seen_sec > 10
        for: 1m
        labels:
          severity: p2
        annotations:
          summary: "Risk replica offline for shard {{ $labels.shard_id }}"
          description: "Replica down. No immediate impact. See RECOVERY-RUNBOOK.md §2.2"

      # P2: MARKETDATA offline
      - alert: MarketdataOffline
        expr: time() - marketdata_heartbeat_last_seen_sec > 10
        for: 1m
        labels:
          severity: p2
        annotations:
          summary: "MARKETDATA offline"
          description: "Market data down. No trading impact. See RECOVERY-RUNBOOK.md §2"
```

---

## 6. Reconciliation Queries

### 6.1 Position Reconciliation (Run Every 10min)

```sql
-- Compare positions vs fills (sample 1% of users)
WITH fills_sum AS (
  SELECT
    taker_user_id AS user_id,
    symbol_id,
    SUM(CASE WHEN side = 0 THEN qty ELSE 0 END) AS fills_buy,
    SUM(CASE WHEN side = 1 THEN qty ELSE 0 END) AS fills_sell
  FROM fills
  WHERE taker_user_id % 100 = 0  -- Sample 1%
  GROUP BY taker_user_id, symbol_id
)
SELECT
  COUNT(*) AS total_checked,
  SUM(CASE WHEN p.long_qty != f.fills_buy OR p.short_qty != f.fills_sell THEN 1 ELSE 0 END) AS mismatches
FROM positions p
JOIN fills_sum f ON p.user_id = f.user_id AND p.symbol_id = f.symbol_id;

-- Export to Prometheus:
-- risk_position_reconciliation_mismatch_total = mismatches
```

### 6.2 Seq Gap Detection (Run Continuously)

```sql
-- Check for seq gaps in fills table
SELECT
  symbol_id,
  COUNT(*) AS gaps
FROM (
  SELECT
    symbol_id,
    seq,
    LAG(seq) OVER (PARTITION BY symbol_id ORDER BY seq) AS prev_seq
  FROM fills
  WHERE seq - LAG(seq) OVER (PARTITION BY symbol_id ORDER BY seq) > 1
) t
GROUP BY symbol_id;

-- Export to Prometheus:
-- me_wal_seq_gap_total{symbol_id} = gaps
```

### 6.3 Tips Monotonic Check (Run Continuously)

```sql
-- Check for tip decreases
SELECT
  instance_id,
  symbol_id,
  COUNT(*) AS violations
FROM (
  SELECT
    instance_id,
    symbol_id,
    last_seq,
    LAG(last_seq) OVER (PARTITION BY instance_id, symbol_id ORDER BY updated_at) AS prev_seq
  FROM tips
  WHERE last_seq < LAG(last_seq) OVER (PARTITION BY instance_id, symbol_id ORDER BY updated_at)
) t
GROUP BY instance_id, symbol_id;

-- Export to Prometheus:
-- risk_tips_decreased_total{instance_id,symbol_id} = violations
```

### 6.4 Funding Zero-Sum Check (Run After Each Settlement)

```sql
-- Check funding payments sum to zero
SELECT
  symbol_id,
  settlement_ts,
  ABS(SUM(amount)) AS abs_sum
FROM funding_payments
GROUP BY symbol_id, settlement_ts
HAVING ABS(SUM(amount)) > 100;  -- Allow 1bps rounding error

-- Export to Prometheus:
-- risk_funding_zero_sum_violations_total{symbol_id} = COUNT(rows)
```

### 6.5 Advisory Lock Check (Run Continuously)

```sql
-- Check advisory lock count per shard
SELECT
  objid AS shard_id,
  COUNT(*) AS lock_count
FROM pg_locks
WHERE locktype = 'advisory'
GROUP BY objid;

-- Export to Prometheus:
-- risk_advisory_lock_count{shard_id} = lock_count
-- (Alert if != 1)
```

---

## 7. Testing Metrics

### 7.1 Chaos Test Metrics

During chaos tests (component kills, partitions), track:

| Metric | Target | Meaning |
|--------|--------|---------|
| Recovery time | <RTO from GUARANTEES.md | Time to resume processing |
| Data loss | 0 fills, <=100ms positions | Verify guarantees hold |
| Invariant violations | 0 | All 8 invariants pass |
| Alert accuracy | 100% | All failures detected |
| Alert latency | <30s | Time from failure to alert |

### 7.2 Load Test Metrics

During sustained load tests (1M fills/sec for 10min), track:

| Metric | Target | Meaning |
|--------|--------|---------|
| ME throughput | >1M fills/sec | Sustained rate |
| WAL flush lag | p99 <10ms | Under load |
| Risk throughput | >1M fills/sec | Sustained rate |
| Postgres write lag | p99 <10ms | Under load |
| Backpressure stalls | 0/sec | No overload |
| Position accuracy | 100% | Reconciliation passes |

---

## 8. Runbook Integration

Every alert MUST reference a runbook section. Alert annotations include:

- **summary:** One-line description of alert
- **description:** Detailed explanation + runbook link

Example:
```yaml
annotations:
  summary: "ME heartbeat timeout for BTCUSD"
  description: "ME may be down. See RECOVERY-RUNBOOK.md §2.1"
```

Ops team receives alert → clicks runbook link → follows step-by-step recovery.

---

This monitoring strategy ensures all guarantees in GUARANTEES.md are
continuously validated and any violation triggers immediate alert + recovery.
