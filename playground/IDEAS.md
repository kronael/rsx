# RSX Playground — Brainstorm

Raw ideas for the playground system, organized into three
pillars: Observe (what you see), Act (what you do), Verify
(what you check).

---

# Observe

Brainstorm of everything to SEE in a dev dashboard for RSX
exchange.

## Process Health

1. **Process table**: pid, binary name, cpu%, mem (RSS),
   uptime, thread count, pinned cores, state
   (running/stopped/crashed)
2. **Core affinity map**: visual grid showing which process
   owns which CPU core (ME pins to dedicated cores)
3. **Restart count**: how many times each process restarted
   since playground start, last restart timestamp
4. **Process dependency graph**: visual showing which
   processes feed which (Gateway->Risk->ME, ME->Marketdata,
   all->Recorder)
5. **SPSC ring backpressure**: per-ring occupancy % (ring
   full = producer stalled), color-coded by severity

## WAL Metrics

6. **Per-process WAL state**: current seq, flush rate
   (records/s), flush latency p50/p99, file count, total
   size on disk
7. **WAL lag dashboard**: for each consumer (Recorder, Risk
   replay, ME replay), show producer seq - consumer seq,
   update every 100ms
8. **WAL rotation status**: current file path, bytes
   written, time until rotation (64MB limit),
   oldest/newest file timestamps
9. **Tip persistence health**: last persisted tip per
   process, age since last tip write (should be <10ms),
   tip file size
10. **WAL dump viewer**: click any WAL file, stream records
    with human-readable fields (price/qty decoded,
    timestamps formatted)

## CMP Metrics

11. **CMP flow table**: per-connection (Gateway->Risk,
    Risk->ME, ME->Marketdata), show sent/recv count, NAK
    count, reorder count, drops, last heartbeat age
12. **CMP sequence gaps**: visual timeline showing gaps
    detected, NAK requests sent, retransmit latency
13. **UDP packet loss rate**: rolling 1-minute window, per
    sender, alert if >0.01%
14. **CMP throughput**: messages/sec sent/recv per
    connection, sparkline for last 60s

## Book State

15. **Live orderbook ladder**: per symbol, 10 levels each
    side, price | size | order count, update every 100ms
16. **BBO strip**: all symbols in grid, bid px | bid qty |
    ask px | ask qty | spread bps, sorted by volume
17. **Book depth chart**: visual area chart showing
    cumulative size at each price level, animates on update
18. **Order count gauge**: active orders in book per symbol,
    max capacity (slab size), color red if >80% full
19. **Book compression stats**: compression map utilization
    %, recentering count, current base price, range covered

## Risk State

20. **Position heatmap**: grid of users x symbols, cell
    color = position size (green long, red short)
21. **Margin ladder**: per user, show collateral | initial
    margin | maint margin | liquidation distance bps
22. **Funding tracking**: per symbol per user, accrued
    funding since last settlement, next settlement
    countdown
23. **Liquidation queue**: users in liquidation, current
    step (mark-only vs full liquidation), time in queue
24. **Risk check latency**: histogram of risk check
    duration, p50/p95/p99/max

## Mark Prices

25. **Mark price table**: per symbol, current mark px,
    staleness (age since last update), source status
26. **Mark price chart**: sparkline for each symbol, last
    5min, overlay index sources with different colors
27. **Source health**: per exchange API, last successful
    poll, error count, rate limit status

## Gateway State

28. **WebSocket connection table**: per connection, user_id,
    remote IP, uptime, messages sent/recv, last activity
29. **Rate limit dashboard**: per user, current token count,
    refill rate, limit hit count, last violation timestamp
30. **Circuit breaker status**: per user, state
    (closed/open/half), failure count, next retry time
31. **Auth failure log**: failed JWT attempts, IP, user_id
    attempted, error reason, timestamp

## Marketdata State

32. **Subscriber table**: per symbol, connection count,
    snapshot depth requested, update rate, last send age
33. **Snapshot cache stats**: per symbol, cache size, last
    rebuild time, rebuild count
34. **Marketdata seq status**: per symbol stream, current
    seq, gap detection count, last gap timestamp

## Postgres State

35. **Connection pool gauge**: active | idle | max
    connections, queue depth, checkout latency p50/p99
36. **Query latency table**: top 10 slowest query types,
    count, p50/p95/p99 latency
37. **Tip persistence writes**: tip update rate
    (writes/sec), write latency, last successful write

## Fills/Trades Stream

38. **Live fill feed**: scrolling list of fills, show
    symbol | side | px | qty | aggressor | latency
39. **Trade aggregation view**: per symbol, rolling 1min
    volume, trade count, avg fill size, last px
40. **Latency annotations**: for each fill, show GW recv
    -> Risk validated -> ME matched -> Fill emitted,
    compute each stage duration

## Log Viewer

41. **Unified log tail**: all processes multiplexed, each
    line prefixed with [process-name], filter by level
42. **Per-process log viewer**: select process from
    dropdown, tail last 1000 lines, search/filter
43. **Error aggregation**: group errors by message pattern,
    show count per pattern, first/last occurrence

## Latency Histogram

44. **End-to-end latency**: GW->ME->GW round-trip,
    histogram with p50/p95/p99/max, per-symbol breakdown
45. **Stage breakdown**: stacked bar chart showing time
    spent in each stage (GW validation, Risk check, CMP
    transit, ME match, CMP return, GW response)

## Timeline/Event Stream

46. **Unified event stream**: all WAL records from all
    processes merged by timestamp, show seq | ts | process
    | event_type | key fields
47. **Event filter/search**: filter by event_type, user_id,
    symbol_id, time range, search by order_id or cid
48. **Causal chain viewer**: click any order_id, show full
    lifecycle: ORDER_ACCEPTED -> FILL(s) -> ORDER_DONE
49. **Anomaly detection**: highlight events with unusual
    latency (>p99), sequence gaps, or invariant violations

## System-Wide

50. **Health score**: single 0-100 gauge aggregating all
    metrics, red/yellow/green by severity
51. **Invariant monitor**: live check of system correctness
    rules, alert on violation with event details

---

# Act

Everything you want to DO from the playground.

## Process Management

1. Start individual process by name
2. Stop individual process (graceful SIGTERM)
3. Restart process with same config
4. Kill process immediately (SIGKILL for crash testing)
5. Show process status (running/stopped/crashed, PID)
6. Tail logs for specific process
7. View resource usage per process (CPU/mem/fd)
8. Hot reload process (restart without dropping connections)

## Scenario Launching

9. Launch minimal scenario (1 gw, 1 risk, 1 ME)
10. Launch full scenario (all components, multiple symbols)
11. Launch stress scenario (high order rate, multi-gateway)
12. Launch custom scenario from config file
13. Launch multi-gateway scenario (load balancing)
14. Launch replication scenario (2 risk shards + failover)
15. Launch cold start scenario (replay from WAL checkpoint)

## State Management

16. Clean tmp directory (wipe all temporary files)
17. Reset database (drop/recreate tables)
18. Wipe WAL logs (delete all .dxs files)
19. Clear all state (tmp + db + WAL in one action)
20. Export current state snapshot (db dump + WAL tip)
21. Import state snapshot (restore from saved checkpoint)

## Fault Injection

22. Kill random process (simulate crash)
23. Pause process via SIGSTOP (freeze without killing)
24. Resume paused process via SIGCONT
25. Network partition (block CMP/UDP between components)
26. Network delay injection (add latency to routes)
27. Corrupt WAL record (test recovery path)
28. Fill disk (simulate out-of-space for WAL rotation)
29. Drop CMP packets (test NACK/retransmit)

## Test Orders

30. Submit single order (market/limit/post-only)
31. Submit batch orders (N orders, same or different users)
32. Submit random order stream (configurable rate)
33. Submit stress burst (1000 orders instant)
34. Submit order sequence (scripted: place, modify, cancel)
35. Submit cross orders (taker + maker, immediate match)
36. Submit self-trade pair (test prevention)
37. Submit invalid orders (test validation)

## User Management

38. Create user (allocate user_id, insert into db)
39. Deposit collateral (add to user balance)
40. Withdraw collateral (test margin check)
41. List users (all user_id, balances, positions)
42. View user state (margin, unrealized PnL, positions)
43. Freeze user (block new orders, keep positions)
44. Unfreeze user (restore trading)
45. Reset user position (close all, zero balance)

## Config Changes

46. Change tick size at runtime
47. Change lot size (min/max qty)
48. Change fee tier (taker/maker fees)
49. Change risk params (initial/maintenance margin ratio)
50. Add new symbol (deploy new ME instance)
51. Remove symbol (gracefully stop ME, archive WAL)
52. Change funding interval (8h to 1h for testing)
53. Change max leverage

## Mark Price Control

54. Trigger mark update (force recalculation)
55. Set manual mark price (override for testing)
56. Simulate mark price swing (gradual change)
57. Configure mark price source
58. List mark price history
59. Set staleness alarm threshold

## Risk Actions

60. Trigger liquidation for user/symbol
61. View liquidation queue
62. Cancel pending liquidation
63. Force position close (market close)
64. View margin details (breakdown per user/symbol)
65. Set liquidation penalty override

## WAL Operations

66. Replay from checkpoint (restart ME/risk from seq)
67. Dump WAL to JSON (binary -> readable)
68. View WAL stats (file count, size, seq range)
69. Compact WAL (merge old files)
70. Stream WAL live (tail new records)
71. Verify WAL integrity (check CRC)
72. Export WAL range (seq N to M)

## Snapshot Operations

73. Save book snapshot to disk
74. Load book snapshot from disk
75. Compare two snapshots (diff)
76. Schedule periodic snapshots
77. List available snapshots
78. Delete old snapshots

## Scaling

79. Add ME instance (new symbol at runtime)
80. Remove ME instance (gracefully)
81. Add gateway instance
82. Remove gateway instance (drain + stop)
83. Add risk shard
84. Rebalance risk shards
85. View cluster topology

---

# Verify

Everything you want to CHECK from the playground.

## Invariant Checking

1. **Fills before ORDER_DONE**: per order, all fill events
   precede ORDER_DONE in WAL seq order
2. **Exactly-one completion**: every order has exactly one
   ORDER_DONE or ORDER_FAILED
3. **FIFO within price level**: orders match in arrival
   order within each price level
4. **Position = sum of fills**: per user per symbol,
   computed position matches risk engine state
5. **Tips monotonic**: WAL tip sequence never decreases
6. **No crossed book**: best bid < best ask always
7. **SPSC FIFO order**: events arrive in ring push order
8. **Slab no-leak**: allocated = free_list + active
9. **Funding zero-sum**: sum of funding payments per
   interval per symbol equals zero
10. **Advisory lock exclusive**: at most one main per shard

## Reconciliation

11. **Frozen margin vs computed**: recompute margin from
    position + open orders, compare to frozen value
12. **Shadow book vs ME book**: marketdata shadow book
    matches ME book at same seq
13. **Mark price vs index**: verify mark = EMA(index)

## WAL Integrity

14. **Monotonic seq, no gaps**: seq[n] = seq[n-1] + 1
15. **Fill symmetry**: taker qty = sum(maker qty) per match
16. **WAL checksum**: verify CRC32 on replay

## Cross-Component Consistency

17. **ME seq vs risk tip**: risk.tip >= ME.seq - capacity
18. **Gateway seq vs ME seq**: seq preserved through path
19. **Marketdata lag**: ME.seq - marketdata.seq < threshold

## Health Checks

20. **Process liveness**: ping /health, expect 200 OK
21. **Ring fullness**: SPSC utilization < 80%
22. **WAL flush lag**: time since last flush < 50ms

## Latency Regression

23. **GW->ME->GW round-trip**: p99 < 100us (baseline 50us)
24. **ME match latency**: p99 < 5us (baseline 500ns)

## Order Lifecycle Tracing

25. **E2E order trace**: given oid, show all events across
    all components as timeline with seq and ts_ns
26. **Stale orders**: orders in ME > 1 hour unfilled

## CMP Delivery Verification

27. **NACK rate**: count NACKs/min, baseline <10/min
28. **Flow control backpressure**: sender - receiver >
    window, flag backpressure events

## E2E Test Scenarios

29. **Full fill**: limit order -> fill -> ORDER_DONE,
    position updated, margin freed
30. **IOC partial fill**: partial fill then cancel, no
    residual in ME book
31. **Liquidation trigger**: force into liquidation zone,
    verify force-close
32. **Funding payment**: advance interval, verify FUNDING
    events sum to zero
33. **WAL replay**: stop ME, replay from tip, shadow book
    matches pre-stop state
34. **Concurrent modifies**: 100 modify requests, all FIFO,
    no lost updates
35. **Rate limit breach**: exceed limit, verify rejection,
    no ME submission
