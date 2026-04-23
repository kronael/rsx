---
status: partial
---

# Telemetry Specification

How RSX processes emit metrics and how they reach
Prometheus.

---

## Philosophy

No Prometheus client libraries in hot-path code. No push
SDKs, no agent sidecars. Processes write structured JSON
to stdout. rsyslog streams logs to files and to Vector.
Vector extracts metrics. Prometheus scrapes Vector.

```
RSX process → stdout → rsyslog → ./log/{process}.log
                              → Vector → :9598/metrics
                                             ↑
                                        Prometheus
```

Hot path does one thing: `tracing::info!` with structured
fields. Everything downstream is the reader's problem.

---

## Emission: Structured Log Lines

Every RSX process writes structured JSON to stdout via
tracing + tracing-subscriber. No file I/O in the process.

### Log Format

```json
{
  "ts_ns": 1738000000123456000,
  "level": "info",
  "target": "rsx_matching::engine",
  "message": "order matched",
  "fields": {
    "symbol_id": 1,
    "latency_ns": 350,
    "ring_full_pct": 12,
    "seq": 12345
  }
}
```

### Metric Fields (per component)

**Gateway**:
- `latency_us`: GW recv to fill sent
- `ws_connections`: active WebSocket count
- `rate_limit_pct`: current rate vs limit
- `circuit_breaker_state`: open/closed/half-open
- `orders_per_sec`: rolling 1s count

**Matching Engine**:
- `match_latency_ns`: per-match cycle time
- `book_depth`: active orders in book
- `compression_pct`: CompressionMap utilization
- `recenter_count`: lifetime recenters
- `seq`: current WAL sequence

**Risk**:
- `margin_check_us`: per-check latency
- `position_count`: active positions
- `liquidation_queue_len`: pending liquidations
- `collateral_total`: sum across users

**Marketdata**:
- `snapshot_lag_seq`: producer seq - shadow seq
- `subscribers`: active SSE/WS connections

**WAL (all processes)**:
- `flush_rate`: records/sec
- `flush_latency_ms`: p50/p99 of flush cycle
- `file_size_bytes`: current file
- `tip_age_ms`: time since last tip persist

**Mark**:
- `mark_px`: per symbol
- `index_deviation_bps`: mark vs index spread
- `funding_rate`: per symbol

---

## Log Transport: rsyslog

Processes write to stdout. rsyslog captures and routes.

### rsyslog Config

```
# /etc/rsyslog.d/rsx.conf

# Template: write raw JSON line
template(name="rawjson" type="string"
  string="%msg:2:$%\n")

# Per-process log files
if $programname startswith 'rsx-' then {
  action(type="omfile"
    dynaFile="rsx-logs"
    template="rawjson")
  # Forward to Vector via unix socket
  action(type="omuxsock"
    socket="/var/run/vector.sock")
  stop
}

# Dynamic file name from program name
template(name="rsx-logs" type="string"
  string="./log/%programname%.log")
```

### Why rsyslog

- Already installed on every Linux box
- Handles log rotation (or logrotate alongside)
- Buffers on disk if Vector is down
- Forwards to Vector via unix socket (no network)
- Process doesn't touch the filesystem

### Process Startup

```bash
# start script launches with logger piping
./target/debug/rsx-gateway 2>&1 | \
  logger -t rsx-gateway --socket-errors=off
```

Or via systemd:
```ini
[Service]
StandardOutput=journal
StandardError=journal
SyslogIdentifier=rsx-gateway
```

---

## Metrics Extraction: Vector

Vector reads from rsyslog (unix socket or file tail),
parses JSON, extracts metrics.

### Vector Config

```toml
[sources.rsx_logs]
type = "file"
include = ["./log/*.log"]
read_from = "end"

[transforms.parse]
type = "remap"
inputs = ["rsx_logs"]
source = '''
. = parse_json!(.message)
'''

[transforms.metrics]
type = "log_to_metric"
inputs = ["parse"]

  [[transforms.metrics.metrics]]
  type = "histogram"
  field = "fields.match_latency_ns"
  name = "me_match_latency_ns"
  tags.symbol_id = "{{fields.symbol_id}}"

  [[transforms.metrics.metrics]]
  type = "histogram"
  field = "fields.latency_us"
  name = "gw_order_latency_us"

  [[transforms.metrics.metrics]]
  type = "gauge"
  field = "fields.ring_full_pct"
  name = "ring_backpressure_pct"
  tags.ring = "{{fields.ring}}"

[sinks.prometheus]
type = "prometheus_exporter"
inputs = ["metrics"]
address = "127.0.0.1:9598"
```

### System Metrics

Vector collects host and DB metrics directly:

```toml
[sources.host]
type = "host_metrics"
collectors = ["disk", "memory", "cpu", "network"]
scrape_interval_secs = 5

[sources.postgres]
type = "postgresql_metrics"
endpoints = [
  "postgresql://localhost:5432/rsx_dev"
]
scrape_interval_secs = 10
```

This provides disk usage/IOPS, DB size/connections,
CPU/memory, network bytes — no custom code.

---

## Storage: Prometheus

Single local Prometheus instance. Scrapes Vector.

### prometheus.yml

```yaml
scrape_configs:
  - job_name: rsx
    scrape_interval: 5s
    static_configs:
      - targets: ["127.0.0.1:9598"]
```

That's it. Grafana optional for dashboards.

### Retention

Default 15 days local storage. No remote write, no
ClickHouse, no S3. If you need historical data, bump
`--storage.tsdb.retention.time`.

---

## Playground Integration

The playground API server (`/api/metrics`) reads log
files directly and aggregates in-memory. It does not
depend on Vector or Prometheus. Vector/Prometheus run
alongside for richer dashboards but are not required.

---

## What Processes Do NOT Do

- No Prometheus client library (`prometheus` crate)
- No push to any collector from hot path
- No metric registration or histogram allocation
- No OpenTelemetry SDK
- No `/metrics` HTTP endpoint per process
- No file I/O for logging (stdout only, rsyslog routes)

---

## Latency Budget

| Stage | Budget |
|-------|--------|
| tracing::info! call | <100ns |
| stdout write | <500ns (buffered) |
| rsyslog → file + socket | <1ms |
| Vector parse + aggregate | ~10ms (batch) |
| Prometheus scrape | 5s interval |

Metrics visible in Prometheus within ~5-15s of emission.
Hot path adds <1us overhead.

---

## Full Pipeline

```
RSX process
  │ stdout (structured JSON)
  ▼
rsyslog
  ├─→ ./log/{process}.log  (file, for playground/grep)
  └─→ unix socket
       ▼
     Vector
       ├─→ parse JSON → extract metrics
       ├─→ host_metrics (disk, cpu, mem, net)
       └─→ postgresql_metrics (db size, conns)
            ▼
          :9598/metrics
            ▼
          Prometheus (localhost:9090)
            ▼
          Grafana (optional, localhost:3001)
```
