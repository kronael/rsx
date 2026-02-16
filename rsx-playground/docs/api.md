# Playground API Reference

The RSX Playground exposes HTTP endpoints for process control, order submission, and system queries. All endpoints return JSON unless noted.

## Base URL

`http://localhost:49171`

## Process Management

### Start All Processes

```
POST /api/processes/all/start
Body: {"scenario": "minimal|duo|full|stress-low|stress-high|stress-ultra"}
```

Builds and starts all processes for the given scenario.

**Response:**
```json
{"status": "ok", "started": ["gateway", "risk", "me_btcusd", ...]}
```

**Notes:**
- Waits for binaries to build (30-60s)
- Starts processes in dependency order
- Returns after all PIDs obtained

### Stop All Processes

```
POST /api/processes/all/stop
```

Stops all running RSX processes (SIGTERM).

**Response:**
```json
{"status": "ok", "stopped": ["gateway", "risk", ...]}
```

### Start Single Process

```
POST /api/processes/{name}/start
```

Starts a single process. Valid names: `gateway`, `risk`, `me_btcusd`, `me_ethusd`, `me_solusd`, `marketdata`, `mark`, `recorder`.

**Response:**
```json
{"status": "ok", "pid": 12345}
```

### Stop Single Process

```
POST /api/processes/{name}/stop
```

Stops a single process (SIGTERM).

**Response:**
```json
{"status": "ok"}
```

### Restart Single Process

```
POST /api/processes/{name}/restart
```

Stops then starts a process.

**Response:**
```json
{"status": "ok", "pid": 12346}
```

### Kill Single Process

```
POST /api/processes/{name}/kill
```

Kills a process (SIGKILL). Use for fault injection.

**Response:**
```json
{"status": "ok"}
```

## Order Management

### Submit Order

```
POST /api/orders/test
Body: {
  "symbol_id": 1,
  "side": "buy|sell",
  "order_type": "limit|market|post_only",
  "price": "50000",
  "qty": "1.0",
  "tif": "GTC|IOC|FOK",
  "user_id": 1,
  "reduce_only": false,
  "post_only": false
}
```

Submits a single test order via WebSocket to Gateway.

**Response:**
```json
{
  "oid": "01JCRA...",
  "status": "submitted|filled|rejected",
  "fills": [{"price": "50000", "qty": "1.0", "side": "buy"}]
}
```

**Notes:**
- Connects to Gateway WebSocket at `ws://localhost:8080`
- Requires JWT token (auto-generated for user_id)
- Returns after ORDER_DONE or timeout (5s)

### Submit Batch Orders

```
POST /api/orders/batch
```

Submits 10 orders (5 buy, 5 sell) on BTC-PERP at different price levels.

**Response:**
```json
{
  "submitted": 10,
  "filled": 8,
  "rejected": 2
}
```

### Submit Random Orders

```
POST /api/orders/random
```

Submits 5 random orders (random symbol/side/price/qty).

**Response:**
```json
{
  "submitted": 5,
  "oids": ["01JCRA...", "01JCRB...", ...]
}
```

### Submit Stress Orders

```
POST /api/stress/run
```

Submits 100 orders rapidly (no delay). Use to test backpressure.

**Response:**
```json
{
  "submitted": 100,
  "rate": "250/s",
  "duration_ms": 400
}
```

### Submit Invalid Order

```
POST /api/orders/invalid
```

Submits an order that violates validation rules (negative price, etc.). Use to test error handling.

**Response:**
```json
{
  "status": "rejected",
  "reason": "invalid price"
}
```

### Cancel Order

```
POST /api/orders/{cid}/cancel
```

Cancels an active order by client ID.

**Response:**
```json
{"status": "ok"}
```

**Notes:**
- Only works for GTC orders (not IOC/FOK)
- Returns immediately (cancel may lag)

## User Management

### Create User

```
POST /api/users
Body: {"initial_balance": 10000}
```

Creates a new user in Risk engine.

**Response:**
```json
{"user_id": 123, "balance": 10000}
```

### Deposit

```
POST /api/users/{user_id}/deposit
Body: {"amount": 1000}
```

Deposits collateral to user account.

**Response:**
```json
{"user_id": 123, "new_balance": 11000}
```

### Freeze User

```
POST /api/risk/users/{user_id}/freeze
```

Freezes user (reject all new orders).

**Response:**
```json
{"status": "ok"}
```

### Unfreeze User

```
POST /api/risk/users/{user_id}/unfreeze
```

Unfreezes user (allow orders again).

**Response:**
```json
{"status": "ok"}
```

## Risk Management

### Trigger Liquidation

```
POST /api/risk/liquidate
Body: {"user_id": 123}
```

Manually triggers liquidation for a user (even if not under margin).

**Response:**
```json
{
  "liquidated": true,
  "positions_closed": 2,
  "pnl": -150
}
```

## System Queries

### Health Check

```
GET /x/health
```

Returns system health score (0-100).

**Response:**
```json
{
  "score": 85,
  "processes_running": 7,
  "processes_total": 8,
  "postgres_ok": true
}
```

### Process List

```
GET /x/processes
```

Returns all process states.

**Response:**
```json
[
  {
    "name": "gateway",
    "state": "running",
    "pid": 12345,
    "cpu": "2.3%",
    "mem": "45MB",
    "uptime": "1h 23m"
  },
  ...
]
```

### Book Snapshot

```
GET /x/book?symbol_id=1
```

Returns orderbook ladder.

**Response:**
```json
{
  "symbol_id": 1,
  "bids": [
    {"price": "49999", "qty": "10.5"},
    {"price": "49998", "qty": "5.2"}
  ],
  "asks": [
    {"price": "50000", "qty": "8.3"},
    {"price": "50001", "qty": "12.1"}
  ],
  "spread": "1"
}
```

### Recent Orders

```
GET /x/recent-orders
```

Returns last 50 orders.

**Response:**
```json
[
  {
    "cid": "abc123",
    "oid": "01JCRA...",
    "symbol": "1",
    "side": "buy",
    "price": "50000",
    "qty": "1.0",
    "status": "filled",
    "ts": "2026-02-13T20:15:30Z"
  },
  ...
]
```

### Order Trace

```
GET /x/order-trace?trace_oid=01JCRA...
```

Returns all WAL events for an order.

**Response:**
```json
[
  {"seq": 123, "type": "ORDER_ACCEPTED", "ts": "..."},
  {"seq": 124, "type": "FILL", "px": "50000", "qty": "1.0"},
  {"seq": 125, "type": "ORDER_DONE", "reason": "filled"}
]
```

### Logs

```
GET /x/logs?log_process=gateway&log_level=error&log_search=order
```

Returns last 1000 log lines matching filters.

**Response:**
```json
[
  "2026-02-13T20:15:30 gateway error order rejected: insufficient margin",
  ...
]
```

**Query params:**
- `log_process`: filter by process name
- `log_level`: filter by level (error/warn/info/debug)
- `log_search`: text search

### WAL Status

```
GET /x/wal-status
```

Returns per-process WAL state.

**Response:**
```json
[
  {
    "name": "me_btcusd",
    "files": 3,
    "total_size": "12MB",
    "newest": "2026-02-13T20:15:00Z"
  },
  ...
]
```

### WAL Files

```
GET /x/wal-files
```

Returns all WAL files on disk.

**Response:**
```json
[
  {
    "stream": "me_btcusd",
    "name": "1_active.wal",
    "size": "4MB",
    "modified": "2026-02-13T20:15:00Z"
  },
  ...
]
```

## Verification

### Run All Checks

```
POST /api/verify/run
```

Runs all 10 invariant checks.

**Response:**
```json
{
  "checks": [
    {"name": "Fills precede ORDER_DONE", "status": "pass"},
    {"name": "Exactly-one completion", "status": "pass"},
    ...
  ]
}
```

### Get Verification Results

```
GET /x/verify
```

Returns last verification results.

**Response:** Same as above.

## WAL Operations

### Verify WAL Integrity

```
POST /api/wal/verify
```

Runs corruption checks on all WAL files.

**Response:**
```json
{
  "status": "ok",
  "files_checked": 12,
  "errors": 0
}
```

### Dump WAL to JSON

```
POST /api/wal/dump
```

Exports all WAL events to JSON (written to `./tmp/wal_dump.json`).

**Response:**
```json
{
  "status": "ok",
  "events": 1234,
  "file": "./tmp/wal_dump.json"
}
```

## Error Handling

All endpoints return HTTP 500 on error with:

```json
{
  "error": "error message",
  "detail": "stack trace or additional context"
}
```

## Notes

- All POST endpoints are idempotent where possible
- Endpoints return immediately (no long polling)
- Use HTMX endpoints (`/x/*`) for partial HTML updates
- Use API endpoints (`/api/*`) for JSON responses
- WebSocket endpoint: `ws://localhost:8080` (Gateway, not playground)
