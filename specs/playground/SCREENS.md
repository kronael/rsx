# RSX Playground — Dashboard Screens

Organizes all ideas from IDEAS.md into 10 dashboard screens
for local dev environment.

---

## Screen 1: Overview

**Purpose**: Single-pane system health with critical metrics.

**Data sources**: GET /api/processes, GET /api/metrics,
Postgres pool stats, WAL tip files.

**Panels**:
- Health score gauge (Obs #50): 0-100, red/yellow/green
- Process table (Obs #1, #3): pid, binary, cpu%, mem, uptime,
  state, restart count
- Key metrics strip: total active orders, open positions,
  messages/sec, WAL lag max across all consumers
- Ring backpressure (Obs #5): visual grid, color-coded by %full
- Invariant monitor (Obs #51): live violation alerts
- Connection pool gauge (Obs #35): active/idle/max, queue depth
- Latency histogram (from GET /api/metrics): optional detail view

**Actions**:
- Start all (via process endpoints)
- Stop all (via process endpoints)
- Note: scenarios and state management via `./start` script
  directly (`./start full`, `./start -c`, `./start --reset-db`)

**Refresh rate**: 1s

**ASCII wireframe**:
```
+------------------------------------------------------------------+
| Health: [==============95============] GREEN      Overview       |
+------------------------------------------------------------------+
| Process Table                                                    |
| PID    Binary      CPU%  Mem   Uptime  Restarts  State          |
| 1234   gateway     12%   45M   10m32s  0         running         |
| 1235   risk        8%    67M   10m31s  1         running         |
| 1236   matching    45%   120M  10m30s  0         running         |
+------------------------------------------------------------------+
| Key Metrics                                                      |
| Active Orders: 1,234  |  Positions: 56  |  Msgs/sec: 12,345     |
| WAL Max Lag: 12 seq   |  Connections: 8  |  Errors: 0           |
+------------------------------------------------------------------+
| Ring Backpressure (% full)                                       |
| GW->Risk: [===     ] 30%   Risk->ME: [=       ] 10%             |
| ME->Mktdata: [=====  ] 50%  ME->Recorder: [==     ] 20%         |
+------------------------------------------------------------------+
| Invariants: All passing | Pool: 5/10 active, 3 queue            |
+------------------------------------------------------------------+
| [Start All] [Stop All]                                         |
+------------------------------------------------------------------+
```

---

## Screen 2: Topology

**Purpose**: Visual map of all processes and CMP connections.

**Data sources**: GET /api/processes (core affinity),
GET /api/metrics (CMP stats).

**Panels**:
- Process dependency graph (Obs #4): visual node+edge diagram
  showing data flow paths
- Core affinity map (Obs #2): CPU core grid with process names
  pinned to cores
- CMP flow table (Obs #11): per-connection sent/recv, NAK,
  drops, heartbeat age
- CMP throughput sparklines (Obs #14): last 60s msgs/sec

**Actions**:
- Add ME instance (Act #79)
- Add gateway instance (Act #81)
- Network partition (Act #25: block connection)
- Network delay injection (Act #26)
- View cluster topology (Act #85)

**Refresh rate**: 1s

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                           Topology                               |
+------------------------------------------------------------------+
| Process Graph                                                    |
|                                                                  |
|  [Gateway] ----CMP/UDP---> [Risk] ----CMP/UDP---> [ME-BTCUSD]   |
|      |                       |                         |         |
|      +-------UDP/CMP---------+                         |         |
|                                                         v         |
|                                               [Marketdata]        |
|                                                         |         |
|  [Recorder] <-----------------WAL/TCP-----------------+          |
+------------------------------------------------------------------+
| Core Affinity:  0:[gateway] 1:[risk] 2:[ME] 3:[mktdata]         |
+------------------------------------------------------------------+
| CMP Connections                                                  |
| Gateway->Risk: 12,345 sent | 12,340 recv | 5 NAK | 0 drop       |
| Risk->ME:      9,876 sent  | 9,876 recv  | 0 NAK | 0 drop       |
| ME->Mktdata:   8,765 sent  | 8,760 recv  | 5 NAK | 0 drop       |
| Sparklines: [msg/s last 60s]                                     |
+------------------------------------------------------------------+
| [Add ME] [Add Gateway] [Network Partition] [Inject Delay]       |
+------------------------------------------------------------------+
```

---

## Screen 3: Book

**Purpose**: Per-symbol orderbook visualization and trades.

**Data sources**: GET /api/books/{symbol_id} (snapshot),
GET /api/events (SSE, filter fills client-side),
compression from ME.

**Panels**:
- Symbol selector dropdown
- Live orderbook ladder (Obs #15): 10 levels bid/ask, price |
  size | order count
- Book depth chart (Obs #17): cumulative area chart (computed
  client-side from level data)
- Order count gauge (Obs #18): active vs max capacity
- Book compression stats (Obs #19): utilization %, recentering
  count, base price, range
- Live fill feed (Obs #38): scrolling list with symbol | side |
  px | qty | aggressor | latency
- Trade aggregation (Obs #39): 1min rolling volume, count, avg
  size, last px

**Actions**:
- Submit order (Act #30): market/limit/post-only form
- Submit batch orders (Act #31)
- Submit cross orders (Act #35: taker + maker)
- Save book snapshot (Act #73)
- Load book snapshot (Act #74)

**Refresh rate**: 100ms

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                    Book — [BTCUSD v]                             |
+------------------------------------------------------------------+
| Orderbook Ladder          | Depth Chart                          |
| ASK                       |              /--------\              |
| 50,100  10.5  (3)         |            /          \              |
| 50,050  5.2   (2)         |          /              \            |
| 50,000  20.0  (5)         |        /                  \          |
| ----------BBO----------   | ------/--------------------\-------- |
| 49,950  15.0  (4)         |      /                      \        |
| 49,900  8.0   (2)         |    /                          \      |
| 49,850  12.5  (3)         |  /                              \    |
| BID                       |                                      |
+------------------------------------------------------------------+
| Orders: 42 / 10,000 (0.4%) | Compression: 85% | Recenters: 3    |
+------------------------------------------------------------------+
| Live Fills                                                       |
| 10:34:26  BTCUSD  BUY   50,000  2.5  TAKER  45us                 |
| 10:34:25  BTCUSD  SELL  50,010  1.0  MAKER  38us                 |
+------------------------------------------------------------------+
| 1min: Vol 125.5 | Trades 23 | Avg 5.5 | Last 50,000             |
+------------------------------------------------------------------+
| [Submit Order] [Batch Orders] [Cross Orders] [Save Snapshot]    |
+------------------------------------------------------------------+
```

---

## Screen 4: Risk

**Purpose**: Per-user positions, margin, and collateral tracking.

**Data sources**: GET /api/risk/users,
GET /api/risk/users/{user_id}, GET /api/risk/liquidations.

**Panels**:
- Position heatmap (Obs #20): users x symbols grid, color by
  position size
- Margin ladder (Obs #21): per user collateral | initial margin
  | maint margin | liquidation distance bps
- Funding tracking (Obs #22): accrued funding per user per
  symbol, next settlement countdown
- Liquidation queue (Obs #23): users in liquidation, current
  step, time in queue
- Risk check latency histogram (Obs #24): p50/p95/p99/max

**Actions**:
- Create user (Act #38: POST /api/users)
- Deposit collateral (Act #39: POST /api/users/{user_id}/deposit)
- View user state (Act #42: GET /api/risk/users/{user_id})
- List all users (Act #41: GET /api/users)
- Freeze user (Act #43: POST /api/risk/users/{user_id}/freeze)
- Unfreeze user (Act #44: POST /api/risk/users/{user_id}/unfreeze)
- Trigger liquidation (Act #60)
- Force position close (Act #63)

**Refresh rate**: 1s

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                           Risk                                   |
+------------------------------------------------------------------+
| Position Heatmap (users x symbols)                               |
|         BTCUSD  ETHUSD  SOLUSD                                   |
| user_1    +5.2    -2.0    0.0     (green/red/gray)               |
| user_2   -10.0    +3.5   +1.0                                    |
| user_3    +2.5    0.0    -0.5                                    |
+------------------------------------------------------------------+
| Margin Ladder (per user)                                         |
| user_1: 10,000 collateral | 3,500 IM | 2,000 MM | 250 bps liq   |
| user_2: 25,000 collateral | 8,900 IM | 5,000 MM | 150 bps liq   |
+------------------------------------------------------------------+
| Funding: user_1 BTCUSD +12.5 accrued | Next: 7h 23m              |
+------------------------------------------------------------------+
| Liquidation Queue: [user_3: mark-only, 12s in queue]            |
+------------------------------------------------------------------+
| Risk Check Latency: p50 5us | p95 12us | p99 23us | max 45us     |
+------------------------------------------------------------------+
| [Create User] [Deposit] [Withdraw] [User Detail] [Liquidate]    |
+------------------------------------------------------------------+
```

---

## Screen 5: WAL

**Purpose**: WAL sequence tracking, lag monitoring, and replay.

**Data sources**: GET /api/wal/{stream}/status,
GET /api/wal/{stream}/events, GET /api/wal/{stream}/stats.

**Panels**:
- Per-process WAL state (Obs #6): current seq, flush rate,
  flush latency p50/p99, file count, total size
- WAL lag dashboard (Obs #7): producer seq - consumer seq for
  Recorder, Risk replay, ME replay, update every 100ms
- WAL rotation status (Obs #8): current file, bytes written,
  time until rotation, oldest/newest timestamps
- Tip persistence health (Obs #9): last persisted tip per
  process, age since write
- Timeline view (Obs #46, #47, #48): unified event stream,
  filter by event_type/user_id/symbol_id, causal chain viewer

**Actions**:
- Replay from checkpoint (Act #66)
- Dump WAL to JSON (Act #67)
- View WAL stats (Act #68)
- Stream WAL live (Act #70)
- Verify WAL integrity (Act #71)
- Export WAL range (Act #72)
- Wipe WAL logs (Act #18)
- WAL dump viewer (Obs #10: click file)

**Refresh rate**: 100ms for lag/tips, 1s for file stats

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                            WAL                                   |
+------------------------------------------------------------------+
| Per-Process State                                                |
| Gateway:  seq 12,345 | 1,234 rec/s | p50 2ms p99 8ms | 3 files  |
| Risk:     seq 12,340 | 1,230 rec/s | p50 3ms p99 9ms | 3 files  |
| ME:       seq 12,338 | 1,228 rec/s | p50 1ms p99 5ms | 3 files  |
+------------------------------------------------------------------+
| Lag Dashboard (producer seq - consumer seq)                      |
| Recorder:   ME 12,338 - Recorder 12,330 = 8 seq lag             |
| Risk replay: Gateway 12,345 - Risk 12,340 = 5 seq lag           |
+------------------------------------------------------------------+
| Rotation: me_00042.dxs | 45/64 MB | 3m until rotation           |
| Tip health: Gateway tip 2ms ago | Risk tip 3ms ago | ME 1ms ago  |
+------------------------------------------------------------------+
| Timeline (last 100 events, filter: [all v])                      |
| 12,345  10:34:26.123456  gateway  ORDER_ACCEPTED  user_1        |
| 12,344  10:34:26.123400  risk     MARGIN_CHECK    user_1        |
| 12,343  10:34:26.123350  ME       FILL            oid_abc        |
+------------------------------------------------------------------+
| [Replay] [Dump JSON] [Stats] [Stream Live] [Verify] [Export]    |
+------------------------------------------------------------------+
```

---

## Screen 6: Logs

**Purpose**: Unified log viewer with filtering and search.

**Data sources**: GET /api/logs, GET /api/events (SSE,
filter for log events client-side).

**Panels**:
- Unified log tail (Obs #41): all processes multiplexed, each
  line prefixed [process-name], filter by level
  (debug/info/warn/error)
- Per-process log viewer (Obs #42): use ?process= filter,
  tail last 1000 lines, search box
- Error aggregation (Obs #43): group errors by message pattern,
  show count, first/last occurrence
- Auth failure log (Obs #31): failed JWT attempts, IP, user_id,
  reason, timestamp

**Actions**:
- Tail logs for process (Act #6)
- Filter by level
- Search/regex filter
- Export logs (download last N lines)
- Clear log view

**Refresh rate**: 1s (tail mode), manual for search

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                           Logs                                   |
+------------------------------------------------------------------+
| Filter: [all processes v] [all levels v] Search: [___________]  |
+------------------------------------------------------------------+
| Unified Log (last 1000 lines)                                    |
| [gateway]   10:34:26 info  websocket connection accepted user_1 |
| [risk]      10:34:26 debug margin check passed user_1 symbol_1  |
| [matching]  10:34:26 info  order matched oid_abc qty 2.5        |
| [gateway]   10:34:26 warn  rate limit near threshold user_2     |
| [recorder]  10:34:26 error failed to write wal: disk full       |
+------------------------------------------------------------------+
| Error Aggregation                                                |
| Pattern: "failed to write wal"  Count: 3  First/Last: 10:34:26  |
| Pattern: "circuit breaker open" Count: 1  First/Last: 10:30:12  |
+------------------------------------------------------------------+
| Auth Failures (last 10)                                          |
| 10:34:20  192.168.1.100  user_5  invalid signature               |
+------------------------------------------------------------------+
| [Export Logs] [Clear View]                                       |
+------------------------------------------------------------------+
```

---

## Screen 7: Control

**Purpose**: Process control and scenario launching.

**Data sources**: GET /api/processes.

**Panels**:
- Process control grid: per process, show status, PID, uptime,
  with start/stop/restart/kill buttons (Act #1-5, #8)
- Resource usage (Act #7): per process cpu/mem/fd gauges

**Actions**:
- Start/stop/restart/kill individual process (Act #1-4)
- Note: scenarios via `./start full|minimal|stress`
- Note: state management via `./start -c` / `./start --reset-db`
- Note: symbol config via config file + process restart

**Refresh rate**: 5s for status/resource, manual for actions

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                          Control                                 |
+------------------------------------------------------------------+
| Process Control                                                  |
| Process     Status     PID    Uptime   [Actions]                 |
| gateway     running    1234   10m32s   [Stop] [Restart] [Kill]  |
| risk        running    1235   10m31s   [Stop] [Restart] [Kill]  |
| matching    running    1236   10m30s   [Stop] [Restart] [Kill]  |
| marketdata  stopped    -      -        [Start]                   |
| recorder    running    1238   10m28s   [Stop] [Restart] [Kill]  |
+------------------------------------------------------------------+
| Note: scenarios via ./start, state via ./start -c/--reset-db    |
+------------------------------------------------------------------+
| Resource Usage                                                   |
| gateway:  CPU 12% [====      ] Mem 45M  FD 23                    |
| risk:     CPU 8%  [===       ] Mem 67M  FD 18                    |
| matching: CPU 45% [=========] Mem 120M  FD 12                    |
+------------------------------------------------------------------+
```

---

## Screen 8: Faults

**Purpose**: Simple fault injection via process control.

**Data sources**: GET /api/processes.

**Panels**:
- Process status list: show which processes are running/stopped
- Quick actions: kill/stop individual processes to simulate
  faults

**Actions**:
- Kill process (via POST /api/processes/{name}/kill)
- Stop process (via POST /api/processes/{name}/stop)
- Restart process (via POST /api/processes/{name}/restart)
- Note: network faults (partition, delay, packet drop) require
  OS-level tools (iptables, tc) run manually
- Note: WAL corruption requires manual hex editing

**Refresh rate**: 1s for process status

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                          Faults                                  |
+------------------------------------------------------------------+
| Process Control (fault injection via kill/stop)                  |
| Process     Status     PID    [Actions]                          |
| gateway     running    1234   [Stop] [Kill]                      |
| risk        running    1235   [Stop] [Kill]                      |
| matching    running    1236   [Stop] [Kill]                      |
| marketdata  running    1237   [Stop] [Kill]                      |
+------------------------------------------------------------------+
| After killing a process, observe recovery via Overview screen.   |
| For network faults: use iptables/tc directly.                    |
+------------------------------------------------------------------+
```

---

## Screen 9: Verify

**Purpose**: Invariant checking and reconciliation.

**Data sources**: GET /api/verify/invariants,
GET /api/verify/invariants/{name}.

**Panels**:
- Invariant monitor: 10 system invariants + reconciliation
  checks with pass/fail status, last check time, violation
  details
- WAL integrity: monotonic seq check, fill symmetry, checksum
- Cross-component consistency: ME seq vs risk tip, gateway seq
  vs ME seq, marketdata lag
- Health checks: process liveness, ring fullness, WAL flush lag
- Latency regression: GW->ME->GW p99, ME match p99 vs baseline

**Actions**:
- Run all checks (trigger full verification)
- Run single check (select from dropdown)
- View violation details (click failed check)
- Export verification report
- Note: E2E tests via `cargo test` directly

**Refresh rate**: 5s for checks, manual for E2E tests

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                          Verify                                  |
+------------------------------------------------------------------+
| Invariants (10 system correctness rules)                         |
| [PASS] Fills before ORDER_DONE           Last: 10:34:25          |
| [PASS] Exactly-one completion             Last: 10:34:25          |
| [PASS] FIFO within price level            Last: 10:34:25          |
| [PASS] Position = sum of fills            Last: 10:34:24          |
| [PASS] Tips monotonic                     Last: 10:34:25          |
| [PASS] No crossed book                    Last: 10:34:25          |
| [FAIL] SPSC FIFO order                    Last: 10:34:23          |
|        Details: event seq 12,340 arrived before 12,339           |
| [PASS] Slab no-leak                       Last: 10:34:25          |
| [PASS] Funding zero-sum                   Last: 10:30:00          |
| [PASS] Advisory lock exclusive            Last: 10:34:24          |
+------------------------------------------------------------------+
| Reconciliation (included in invariants)                          |
| Frozen margin vs computed:  [PASS]  Last: 10:34:20               |
| Shadow book vs ME book:     [PASS]  Last: 10:34:21               |
| Mark price vs index:        [PASS]  Last: 10:34:22               |
+------------------------------------------------------------------+
| Latency Regression (vs baseline)                                 |
| GW->ME->GW p99: 48us (baseline 50us) [PASS]                     |
| ME match p99:   450ns (baseline 500ns) [PASS]                   |
+------------------------------------------------------------------+
| [Run All Checks] [Run: FIFO check v] [View Details] [Export]    |
| E2E tests: use `cargo test` directly                            |
+------------------------------------------------------------------+
```

---

## Screen 10: Orders

**Purpose**: Test order submission and lifecycle tracing.

**Data sources**: POST /api/orders/test,
GET /api/orders/{oid}/trace.

**Panels**:
- Order submission form (Act #30-37): market/limit/post-only,
  symbol, side, price, qty, user dropdown
- Batch orders: submit multiple via repeated /api/orders/test
- Random order stream (Act #32): configurable rate, distribution
- Order lifecycle tracer (Ver #25): given oid, show full event
  timeline across all components with seq and ts_ns
- Latency annotations (Obs #40): per fill show GW recv -> Risk
  validated -> ME matched -> Fill emitted, stage durations
- Stale orders (Ver #26): orders in ME >1 hour unfilled
- Order status table: recent orders with cid, oid, symbol,
  side, price, qty, status, latency

**Actions**:
- Submit single order (Act #30)
- Submit batch orders (Act #31)
- Submit random order stream (Act #32)
- Submit stress burst (Act #33: 1000 orders instant)
- Submit order sequence (Act #34: place, modify, cancel)
- Submit cross orders (Act #35)
- Submit self-trade pair (Act #36: test prevention)
- Submit invalid orders (Act #37: test validation)
- Trace order lifecycle (click oid)
- Cancel order
- Modify order

**Refresh rate**: Real-time for submission, 100ms for lifecycle
updates

**ASCII wireframe**:
```
+------------------------------------------------------------------+
|                          Orders                                  |
+------------------------------------------------------------------+
| Submit Order                                                     |
| Symbol: [BTCUSD v] Side: [BUY v] Type: [LIMIT v]                |
| Price: [50000] Qty: [2.5] User: [user_1 v] [Submit Order]       |
| [Batch Orders] [Random Stream] [Stress Burst] [Invalid Orders]  |
+------------------------------------------------------------------+
| Order Lifecycle Trace (oid: abc-def-123)                         |
| seq     ts_ns          component  event            latency       |
| 12,340  10:34:26.100   gateway    ORDER_RECV       -             |
| 12,341  10:34:26.125   risk       MARGIN_CHECK     25us          |
| 12,342  10:34:26.145   ME         ORDER_ACCEPTED   20us          |
| 12,343  10:34:26.158   ME         FILL             13us          |
| 12,344  10:34:26.165   gateway    FILL_SENT        7us           |
| Total: 65us (GW->ME->GW)                                         |
+------------------------------------------------------------------+
| Recent Orders (last 50)                                          |
| cid        oid          symbol  side  px      qty   status  lat  |
| cli_001    abc-def-123  BTCUSD  BUY   50,000  2.5   FILLED  65us |
| cli_002    def-ghi-456  ETHUSD  SELL  3,000   5.0   OPEN    45us |
+------------------------------------------------------------------+
| Stale Orders (>1 hour unfilled): 0                               |
+------------------------------------------------------------------+
```

---

## Cross-References

All ideas from IDEAS.md are mapped to screens:

### Observe
- Process: Screen 1 (health), Screen 2 (topology), Screen 7 (control)
- WAL: Screen 5 (dedicated)
- CMP: Screen 2 (topology), Screen 9 (verify CMP delivery)
- Book: Screen 3 (dedicated)
- Risk: Screen 4 (dedicated)
- Mark: Screen 4 (mark price funding)
- Gateway: Screen 6 (auth failures), Screen 9 (rate limits)
- Marketdata: Screen 3 (snapshot), Screen 5 (seq status)
- Postgres: Screen 1 (connection pool)
- Fills/Trades: Screen 3 (live feed), Screen 10 (latency
  annotations)
- Logs: Screen 6 (dedicated)
- Latency: Screen 10 (stage breakdown), Screen 9 (regression)
- Timeline: Screen 5 (WAL timeline), Screen 10 (order lifecycle)
- System-Wide: Screen 1 (health score, invariants)

### Act
- Process Management: Screen 7 (dedicated control panel)
- Scenario Launching: Screen 7 (launch dropdown)
- State Management: Screen 7 (clean/reset/wipe)
- Fault Injection: Screen 8 (dedicated)
- Test Orders: Screen 10 (dedicated)
- User Management: Screen 4 (create/deposit/withdraw)
- Config Changes: Screen 7 (via process restart with new config)
- Mark Price Control: Screen 4 (trigger update, manual override)
- Risk Actions: Screen 4 (liquidation, force close)
- WAL Operations: Screen 5 (replay, dump, verify)
- Snapshot Operations: Screen 3 (save/load book snapshot)
- Scaling: Screen 2 (add/remove instances)

### Verify
- Invariant Checking: Screen 9 (dedicated, 10 rules)
- Reconciliation: Screen 9 (margin, book, mark)
- WAL Integrity: Screen 9 (seq, symmetry, checksum)
- Cross-Component Consistency: Screen 9 (seq tracking)
- Health Checks: Screen 9 (liveness, ring, flush)
- Latency Regression: Screen 9 (baselines)
- Order Lifecycle Tracing: Screen 10 (e2e trace)
- CMP Delivery Verification: Screen 9 (NACK rate, backpressure)
- E2E Test Scenarios: Screen 9 (full fill, IOC, liquidation,
  funding, replay, modifies, rate limit)

---

## Implementation Notes

- All screens use standard web tech (HTML/CSS/JS + SSE
  for real-time updates)
- Data fetched via REST API endpoints plus SSE streams
  for live updates
- Postgres queries for historical data (positions, orders)
- WAL streams consumed via DXS client for event timelines
- ASCII wireframes are templates; actual implementation uses
  proper UI components
- Refresh rates are defaults; all screens support manual refresh
- Filter/search uses client-side filtering for <1000 items,
  server-side for larger datasets
- Export actions dump to JSON or CSV for external analysis
- All actions require confirmation for destructive operations
  (kill, wipe, reset)
