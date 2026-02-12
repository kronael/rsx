# RSX Playground REST API Specification

Local-only dev tool. No authentication. Wraps Python start script,
Postgres queries, WAL file access, and process health endpoints.

---

## 1. Processes

### GET /api/processes

List all RSX processes with status.

**Query params**: None

**Response** (200):
```json
{
  "processes": [
    {
      "name": "gateway",
      "pid": 1234,
      "status": "running",
      "cpu_percent": 12.5,
      "mem_bytes": 47185920,
      "uptime_secs": 632,
      "restart_count": 0,
      "core_affinity": [0],
      "health_status": "healthy"
    },
    {
      "name": "risk",
      "pid": 1235,
      "status": "running",
      "cpu_percent": 8.2,
      "mem_bytes": 70254592,
      "uptime_secs": 631,
      "restart_count": 1,
      "core_affinity": [1],
      "health_status": "healthy"
    },
    {
      "name": "matching-BTCUSD",
      "pid": 1236,
      "status": "running",
      "cpu_percent": 45.0,
      "mem_bytes": 125829120,
      "uptime_secs": 630,
      "restart_count": 0,
      "core_affinity": [2],
      "health_status": "degraded"
    },
    {
      "name": "marketdata",
      "pid": null,
      "status": "stopped",
      "cpu_percent": 0.0,
      "mem_bytes": 0,
      "uptime_secs": 0,
      "restart_count": 0,
      "core_affinity": [],
      "health_status": "down"
    }
  ],
  "system_health_score": 95
}
```

**Errors**:
- 500: Failed to query process registry

**Data source**: Python process manager state, /proc/[pid]/stat

**Notes**: Refresh every 1s for overview screen

---

### POST /api/processes/{name}/start

Start a process.

**Path params**:
- `name`: Process name (gateway, risk, matching-BTCUSD, etc.)

**Request body**: None

**Response** (200):
```json
{
  "name": "gateway",
  "pid": 1234,
  "status": "starting"
}
```

**Errors**:
- 400: Process already running
- 404: Unknown process name
- 500: Failed to start process

**Data source**: Python start script

---

### POST /api/processes/{name}/stop

Stop a process (SIGTERM).

**Path params**:
- `name`: Process name

**Request body**: None

**Response** (200):
```json
{
  "name": "gateway",
  "status": "stopped"
}
```

**Errors**:
- 404: Unknown process or not running
- 500: Failed to stop process

**Data source**: Python start script

---

### POST /api/processes/{name}/kill

Kill a process (SIGKILL).

**Path params**:
- `name`: Process name

**Request body**: None

**Response** (200):
```json
{
  "name": "gateway",
  "status": "killed"
}
```

**Errors**:
- 404: Unknown process or not running
- 500: Failed to kill process

**Data source**: Python start script

---

### POST /api/processes/{name}/restart

Restart a process (stop then start).

**Path params**:
- `name`: Process name

**Request body**: None

**Response** (200):
```json
{
  "name": "gateway",
  "pid": 1234,
  "status": "restarting"
}
```

**Errors**:
- 404: Unknown process
- 500: Failed to restart process

**Data source**: Python start script

---

## 2. Books

### GET /api/books

List all symbols with BBO strip.

**Query params**: None

**Response** (200):
```json
{
  "symbols": [
    {
      "symbol_id": 1,
      "symbol_name": "BTCUSD",
      "bid_px": 4995000000000,
      "bid_qty": 1500000000,
      "ask_px": 5000000000000,
      "ask_qty": 2000000000,
      "last_px": 4995000000000,
      "last_qty": 500000000,
      "last_ts_ns": 1738000000000000000,
      "order_count": 42,
      "volume_24h": 125500000000
    },
    {
      "symbol_id": 2,
      "symbol_name": "ETHUSD",
      "bid_px": 299500000000,
      "bid_qty": 5000000000,
      "ask_px": 300000000000,
      "ask_qty": 3500000000,
      "last_px": 299500000000,
      "last_qty": 1000000000,
      "last_ts_ns": 1738000001000000000,
      "order_count": 28,
      "volume_24h": 34200000000
    }
  ]
}
```

**Errors**:
- 500: Failed to query marketdata

**Data source**: Marketdata /snapshot endpoint

**Notes**: Refresh every 100ms for book screen

---

### GET /api/books/{symbol_id}

Full orderbook snapshot.

**Path params**:
- `symbol_id`: Symbol ID (u32)

**Response** (200):
```json
{
  "symbol_id": 1,
  "symbol_name": "BTCUSD",
  "seq": 12345,
  "ts_ns": 1738000000000000000,
  "bids": [
    {
      "px": 4995000000000,
      "qty": 1500000000,
      "order_count": 4
    },
    {
      "px": 4990000000000,
      "qty": 800000000,
      "order_count": 2
    }
  ],
  "asks": [
    {
      "px": 5000000000000,
      "qty": 2000000000,
      "order_count": 5
    },
    {
      "px": 5005000000000,
      "qty": 520000000,
      "order_count": 2
    }
  ],
  "orders": [
    {
      "oid": "0192d5f0-1234-7890-abcd-ef1234567890",
      "side": "BUY",
      "px": 4995000000000,
      "qty": 500000000,
      "filled_qty": 0,
      "user_id": 1
    }
  ],
  "compression": {
    "base_px": 5000000000000,
    "range_ticks": 2000,
    "utilization_percent": 85,
    "recenter_count": 3
  }
}
```

**Errors**:
- 404: Unknown symbol
- 500: Failed to query book

**Data source**: Matching engine /state endpoint or marketdata
shadow book

**Notes**: Includes CompressionMap stats. Client computes
cumulative depth from levels if needed.

---

## 3. Risk

### GET /api/risk/users/{user_id}

Detailed user risk state.

**Path params**:
- `user_id`: User ID (u32)

**Response** (200):
```json
{
  "user_id": 1,
  "collateral": 10000000000000,
  "initial_margin": 3500000000000,
  "maint_margin": 2000000000000,
  "unrealized_pnl": 125000000000,
  "realized_pnl": 50000000000,
  "liquidation_distance_bps": 250,
  "frozen": false,
  "positions": [
    {
      "symbol_id": 1,
      "symbol_name": "BTCUSD",
      "qty": 520000000,
      "entry_px": 4980000000000,
      "mark_px": 5000000000000,
      "unrealized_pnl": 104000000,
      "funding_accrued": 12500000
    },
    {
      "symbol_id": 2,
      "symbol_name": "ETHUSD",
      "qty": -200000000,
      "entry_px": 300000000000,
      "mark_px": 299500000000,
      "unrealized_pnl": 10000000,
      "funding_accrued": -3200000
    }
  ],
  "margin_breakdown": {
    "im_rate": 0.10,
    "mm_rate": 0.05,
    "position_notional": 35000000000000,
    "order_notional": 5000000000000
  }
}
```

**Errors**:
- 404: Unknown user
- 500: Failed to query risk state

**Data source**: Postgres (positions, balances) + Risk /state

**Notes**: Includes per-position funding accrual

---

### GET /api/risk/liquidations

Liquidation queue.

**Query params**: None

**Response** (200):
```json
{
  "liquidations": [
    {
      "user_id": 3,
      "symbol_id": 1,
      "position_qty": -1000000000,
      "liquidation_step": "mark-only",
      "started_ts_ns": 1738000000000000000,
      "elapsed_secs": 12,
      "last_reduce_qty": 0
    }
  ]
}
```

**Errors**:
- 500: Failed to query liquidation queue

**Data source**: Risk engine /liquidations endpoint

**Notes**: Empty array if no active liquidations

---

### POST /api/risk/users/{user_id}/freeze

Freeze user (reject new orders).

**Path params**:
- `user_id`: User ID (u32)

**Request body**: None

**Response** (200):
```json
{
  "user_id": 1,
  "frozen": true
}
```

**Errors**:
- 404: Unknown user
- 500: Failed to freeze user

**Data source**: Postgres (users table)

---

### POST /api/risk/users/{user_id}/unfreeze

Unfreeze user.

**Path params**:
- `user_id`: User ID (u32)

**Request body**: None

**Response** (200):
```json
{
  "user_id": 1,
  "frozen": false
}
```

**Errors**:
- 404: Unknown user
- 500: Failed to unfreeze user

**Data source**: Postgres (users table)

---

## 4. WAL

### GET /api/wal/{stream}/status

WAL stream status.

**Path params**:
- `stream`: Stream name (gateway, risk, matching-BTCUSD, etc.)

**Response** (200):
```json
{
  "stream": "matching-BTCUSD",
  "current_seq": 12345,
  "flush_rate": 1234,
  "flush_latency_p50_ms": 2,
  "flush_latency_p99_ms": 8,
  "file_count": 3,
  "total_size_bytes": 201326592,
  "current_file": "me_btcusd_00042.dxs",
  "current_file_bytes": 47185920,
  "rotation_in_secs": 180,
  "tip_age_ms": 2
}
```

**Errors**:
- 404: Unknown stream
- 500: Failed to read WAL state

**Data source**: WAL tip files (./tmp/wal/[stream].tip),
file metadata

**Notes**: Refresh every 100ms for WAL screen. Includes
file_count, total_size, seq range (previously in /stats).

---

### GET /api/wal/{stream}/events

Retrieve WAL records.

**Path params**:
- `stream`: Stream name

**Query params**:
- `from`: Start seq (inclusive, default: 0)
- `limit`: Max records (default: 100, max: 10000)

**Response** (200):
```json
{
  "stream": "matching-BTCUSD",
  "events": [
    {
      "seq": 12345,
      "ts_ns": 1738000000123456000,
      "event_type": "ORDER_ACCEPTED",
      "payload": {
        "oid": "0192d5f0-1234-7890-abcd-ef1234567890",
        "user_id": 1,
        "symbol_id": 1,
        "side": "BUY",
        "px": 5000000000000,
        "qty": 250000000,
        "order_type": "LIMIT"
      }
    },
    {
      "seq": 12346,
      "ts_ns": 1738000000123500000,
      "event_type": "FILL",
      "payload": {
        "oid": "0192d5f0-1234-7890-abcd-ef1234567890",
        "taker_user_id": 1,
        "maker_user_id": 2,
        "symbol_id": 1,
        "px": 5000000000000,
        "qty": 250000000,
        "side": "BUY",
        "aggressor": "TAKER"
      }
    }
  ],
  "total_records": 12346,
  "has_more": false
}
```

**Errors**:
- 404: Unknown stream
- 400: Invalid seq or limit
- 500: Failed to read WAL

**Data source**: WAL files (./tmp/wal/[stream]_*.dxs)

**Notes**: Events decoded from binary WAL format

---

## 5. Mark

### GET /api/mark/prices

All mark prices.

**Query params**: None

**Response** (200):
```json
{
  "prices": [
    {
      "symbol_id": 1,
      "symbol_name": "BTCUSD",
      "mark_px": 5000000000000,
      "index_px": 5000500000000,
      "funding_rate": 12500,
      "next_funding_ts_ns": 1738003600000000000
    },
    {
      "symbol_id": 2,
      "symbol_name": "ETHUSD",
      "mark_px": 299500000000,
      "index_px": 299600000000,
      "funding_rate": 8300,
      "next_funding_ts_ns": 1738003600000000000
    }
  ]
}
```

**Errors**:
- 500: Failed to query mark prices

**Data source**: Mark price aggregator /prices endpoint

**Notes**: Funding rate in basis points (100 bps = 0.01%)

---

## 6. Orders

### POST /api/orders/test

Submit single test order.

**Request body**:
```json
{
  "user_id": 1,
  "symbol_id": 1,
  "side": "BUY",
  "order_type": "LIMIT",
  "px": 5000000000000,
  "qty": 250000000,
  "post_only": false,
  "reduce_only": false,
  "cid": "client-order-001"
}
```

**Response** (200):
```json
{
  "oid": "0192d5f0-1234-7890-abcd-ef1234567890",
  "cid": "client-order-001",
  "status": "ACCEPTED",
  "ts_ns": 1738000000000000000
}
```

**Errors**:
- 400: Invalid order parameters
- 404: Unknown user or symbol
- 429: Rate limit exceeded
- 500: Failed to submit order

**Data source**: Gateway WebSocket API (internal submission)

**Notes**: Bypasses WebSocket auth for test orders

---

### GET /api/orders/{oid}/trace

Order lifecycle trace.

**Path params**:
- `oid`: Order ID (UUIDv7 string)

**Response** (200):
```json
{
  "oid": "0192d5f0-1234-7890-abcd-ef1234567890",
  "cid": "client-order-001",
  "user_id": 1,
  "symbol_id": 1,
  "trace": [
    {
      "seq": 12340,
      "ts_ns": 1738000000100000000,
      "component": "gateway",
      "event": "ORDER_RECV",
      "latency_us": null
    },
    {
      "seq": 12341,
      "ts_ns": 1738000000125000000,
      "component": "risk",
      "event": "MARGIN_CHECK",
      "latency_us": 25
    },
    {
      "seq": 12342,
      "ts_ns": 1738000000145000000,
      "component": "matching",
      "event": "ORDER_ACCEPTED",
      "latency_us": 20
    },
    {
      "seq": 12343,
      "ts_ns": 1738000000158000000,
      "component": "matching",
      "event": "FILL",
      "latency_us": 13,
      "details": {
        "px": 5000000000000,
        "qty": 250000000,
        "aggressor": "TAKER"
      }
    },
    {
      "seq": 12344,
      "ts_ns": 1738000000165000000,
      "component": "gateway",
      "event": "FILL_SENT",
      "latency_us": 7
    }
  ],
  "total_latency_us": 65,
  "final_status": "FILLED"
}
```

**Errors**:
- 404: Unknown order
- 500: Failed to query WAL streams

**Data source**: WAL streams (all components), correlated
by oid

**Notes**: Scans WAL files for matching events

---

## 7. Verify

### GET /api/verify/invariants

Run all invariant checks (includes reconciliation).

**Query params**: None

**Response** (200):
```json
{
  "checks": [
    {
      "name": "fills_before_order_done",
      "status": "PASS",
      "last_check_ts_ns": 1738000000000000000,
      "details": null
    },
    {
      "name": "exactly_one_completion",
      "status": "PASS",
      "last_check_ts_ns": 1738000000000000000,
      "details": null
    },
    {
      "name": "fifo_within_price_level",
      "status": "FAIL",
      "last_check_ts_ns": 1738000000000000000,
      "details": {
        "violation": "order seq 12340 filled before 12339",
        "symbol_id": 1,
        "px": 5000000000000
      }
    },
    {
      "name": "frozen_margin_vs_computed",
      "status": "PASS",
      "last_check_ts_ns": 1738000000000000000,
      "details": null
    },
    {
      "name": "shadow_book_vs_me_book",
      "status": "PASS",
      "last_check_ts_ns": 1738000000000000000,
      "details": null
    }
  ],
  "summary": {
    "total": 13,
    "passed": 12,
    "failed": 1
  }
}
```

**Errors**:
- 500: Failed to run checks

**Data source**: WAL streams, Postgres, process /state

**Notes**: Expensive operation, runs for ~5-10s. Includes
the 10 system invariants plus reconciliation checks
(margin, shadow book, mark price).

---

### GET /api/verify/invariants/{name}

Run specific invariant check.

**Path params**:
- `name`: Invariant name (fills_before_order_done,
  exactly_one_completion, fifo_within_price_level,
  frozen_margin_vs_computed, shadow_book_vs_me_book, etc.)

**Response** (200):
```json
{
  "name": "fifo_within_price_level",
  "status": "PASS",
  "last_check_ts_ns": 1738000000000000000,
  "details": null
}
```

**Errors**:
- 404: Unknown invariant
- 500: Failed to run check

**Data source**: WAL streams, process state

---

## 8. Metrics

### GET /api/metrics

Overall system metrics.

**Query params**: None

**Response** (200):
```json
{
  "latency": {
    "gw_to_me_to_gw_us": {
      "p50": 45,
      "p95": 65,
      "p99": 85,
      "max": 120
    },
    "me_match_ns": {
      "p50": 350,
      "p95": 450,
      "p99": 480,
      "max": 520
    }
  },
  "throughput": {
    "orders_per_sec": 12345,
    "fills_per_sec": 8765,
    "messages_per_sec": 45678
  },
  "ring_backpressure": {
    "gw_to_risk": 30,
    "risk_to_me": 10,
    "me_to_mktdata": 50,
    "me_to_recorder": 20
  }
}
```

**Errors**:
- 500: Failed to query metrics

**Data source**: Process /metrics endpoints, structured logs

**Notes**: Aggregated from last 60s. Includes latency
percentiles (previously in /metrics/latency).

---

## 9. Logs

### GET /api/logs

Query logs with filters.

**Query params**:
- `process`: Filter by process (optional)
- `level`: Filter by level (optional)
- `tail`: Number of lines (default: 100, max: 10000)
- `search`: Text search (optional)

**Response** (200):
```json
{
  "logs": [
    {
      "process": "gateway",
      "ts_ns": 1738000000000000000,
      "level": "info",
      "message": "websocket connection accepted",
      "fields": {
        "user_id": 1,
        "remote_addr": "127.0.0.1:54321"
      }
    },
    {
      "process": "risk",
      "ts_ns": 1738000001000000000,
      "level": "debug",
      "message": "margin check passed",
      "fields": {
        "user_id": 1,
        "symbol_id": 1
      }
    }
  ],
  "total_lines": 1234,
  "filtered_lines": 56
}
```

**Errors**:
- 400: Invalid query params
- 500: Failed to query logs

**Data source**: Structured log files, process stdout

**Notes**: Search is case-insensitive substring match.
Filter by process replaces the old per-process logs
endpoint.

---

## 10. Events (SSE)

### GET /api/events

Server-sent events stream.

**Query params**: None

**Response**: SSE stream

**Event types**:
- `fill`: Fill events
- `order`: Order lifecycle events
- `health`: Process health changes
- `wal`: WAL record events

**Example events**:
```
event: fill
data: {"seq":12345,"ts_ns":1738000000000000000,
      "symbol_id":1,"px":5000000000000,"qty":250000000,
      "side":"BUY","aggressor":"TAKER"}

event: health
data: {"process":"gateway","status":"degraded",
      "reason":"high_latency"}

event: wal
data: {"stream":"matching-BTCUSD","seq":12346,
      "event_type":"ORDER_ACCEPTED"}
```

**Errors**:
- 500: Failed to establish SSE stream

**Data source**: Live WAL streams, process health endpoints

**Notes**: Client reconnects on disconnect. Filter by
event type client-side (replaces /events/fills and
/events/logs).

---

## Data Types

### Common Types

All timestamps: `ts_ns` (i64 nanoseconds since Unix epoch)

All prices: `px` (i64 fixed-point, smallest unit)

All quantities: `qty` (i64 fixed-point, smallest unit)

All user IDs: `user_id` (u32)

All symbol IDs: `symbol_id` (u32)

All order IDs: `oid` (UUIDv7 string, 16 bytes)

All client IDs: `cid` (20-char string)

All sequences: `seq` (u64)

Side enum: "BUY" | "SELL"

Order type enum: "LIMIT" | "MARKET" | "POST_ONLY"

Order status: "ACCEPTED" | "FILLED" | "PARTIALLY_FILLED" |
"CANCELLED" | "REJECTED"

Process status: "running" | "stopped" | "starting" |
"stopping" | "crashed"

Health status: "healthy" | "degraded" | "down"

---

## Error Response Format

All errors return:
```json
{
  "error": {
    "code": "INVALID_PARAMETER",
    "message": "symbol_id must be positive integer",
    "details": {
      "field": "symbol_id",
      "value": -1
    }
  }
}
```

Common error codes:
- `INVALID_PARAMETER`: Bad request parameter
- `NOT_FOUND`: Resource not found
- `CONFLICT`: Resource conflict (already exists)
- `RATE_LIMIT`: Rate limit exceeded
- `INTERNAL_ERROR`: Server error

---

## Implementation Notes

### Process Management
- Python start script manages all process lifecycle
- PID files in ./tmp/pids/
- Process registry in-memory (start script state)
- Health checks via HTTP /health endpoints

### WAL Access
- WAL files in ./tmp/wal/[stream]_*.dxs
- Tip files in ./tmp/wal/[stream].tip
- Binary format requires parsing library (rsx-dxs)
- API server embeds DxsReader for decoding

### Postgres
- Connection pool (5-10 connections)
- Tables: users, balances, positions, symbol_config, orders
- Migrations in rsx-risk/migrations/

### Metrics Collection
- Parse structured logs from ./log/[process].log
- Poll /metrics endpoints on processes
- Aggregate in-memory (rolling 60s window)
- No Prometheus integration (use structured logs)

### Real-time Updates
- SSE for streaming data (fills, logs, health)
- HTTP polling fallback (1s interval)

### Security
- Local-only (127.0.0.1 bind)
- No authentication (dev tool)
- CORS disabled
- Rate limiting optional (dev mode)

### Performance
- Response cache for expensive queries (invariants,
  WAL scans)
- Stream pagination for large datasets (WAL events)
- Index on Postgres tables (user_id, symbol_id, oid)

### What's NOT in the API

These are handled by existing tools:

- **Scenarios/state**: Use `./start` script directly
  (`./start full`, `./start -c`, `./start --reset-db`)
- **Users/deposits**: Direct SQL (`INSERT INTO users`,
  `UPDATE balances`)
- **Symbol config**: Edit config file + restart process
- **Fault injection**: Use process kill/stop endpoints
  for simple faults; OS-level tools (iptables, tc) for
  network faults
- **E2E tests**: Use `cargo test` directly
- **Snapshots**: Not yet implemented

---

## Endpoint Summary

Total: 22 endpoints

| Group | Count | Purpose |
|-------|-------|---------|
| Processes | 5 | List, start, stop, kill, restart |
| Books | 2 | BBO strip, full snapshot |
| Risk | 4 | User detail, liquidations, freeze/unfreeze |
| WAL | 2 | Status, events |
| Mark | 1 | All prices |
| Orders | 2 | Test order, lifecycle trace |
| Verify | 2 | All invariants, single invariant |
| Metrics | 1 | Latency + throughput + backpressure |
| Logs | 1 | Query with filters |
| Events | 1 | SSE unified stream |

---

## Version

API Version: v1

Spec Version: 2026-02-11

Codebase: RSX Exchange (github.com/onvos/rsx)

---
