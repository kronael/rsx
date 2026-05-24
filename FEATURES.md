# FEATURES.md

Feature inventory for the RSX perpetuals exchange.

## System Overview

Spec-first perpetuals exchange with separate processes
communicating via casting (C structs over UDP) and WAL
replication (TCP). Target: <50us GW->ME->GW round trip.

## Process Architecture

| Process   | Port | Role                                    |
|-----------|------|-----------------------------------------|
| Gateway   | 8080 | WS ingress, JWT auth, rate limit        |
| Risk      | UDP  | Per-shard margin, liquidation, funding  |
| Matching  | SPSC | Per-symbol, slab alloc, price-time FIFO |
| Marketdata| 8180 | Shadow book, L2/BBO/trades broadcast    |
| Mark      | 9201 | Binance/Coinbase aggregation, staleness |
| Recorder  | replication  | Archival consumer, daily rotation       |
| Maker     | WS   | Two-sided quoting, auto-reconnect       |

## Crate Features

### rsx-types

- Price(i64), Qty(i64) newtypes (#[repr(transparent)])
- Side, TIF (GTC/IOC/FOK), FinalStatus enums
- SymbolConfig (tick_size, lot_size, symbol_id)
- validate_order() pre-validation
- install_panic_handler(), time utils

### rsx-book

- Slab arena allocator for fixed-size OrderSlots
- PriceLevel linked lists (bid desc, ask asc)
- CompressionMap for sparse price ranges
- Snapshot save/load (binary)
- Book migration between configs
- Event buffer: fills, done, cancel events
- User state tracking per order
- Matching algorithm: price-time FIFO

### rsx-matching

- Wire format deserialization
- DedupTracker (reject duplicate oid/cid)
- BBO derivation after each match cycle
- WAL integration (write fills, done, BBO)
- CONFIG_APPLIED handling
- Message encoding for outbound casting
- O(1) `(user_id, oid)` cancel index (no linear book scan)

### rsx-dxs (transport, domain-agnostic)

- WalWriter: 10ms flush, 64MB rotate, 10min retain
- WalReader with sequence extraction
- DxsReplayService (TCP, from seq N)
- Streaming protocol (casting) over UDP: flow control, NACK-based
- Two-tier NAK retransmit: in-mem ring + WAL random-access
- Protocol records: StatusMessage, Nak, CmpHeartbeat,
  ReplayRequest, CaughtUpRecord (in `protocol.rs`)
- TLS support, backpressure, tip persistence
- Wire-format version byte in `WalHeader` (V0/V1); readers
  reject unknown versions, writers stamp the current one
- Preallocated `send_ring` on casting sender (zero heap on the
  send path)
- No `rsx-types` dep — transport accepts any
  `CmpRecord` (repr(C) + seq at offset 0)

### rsx-messages (exchange wire records)

- Extracted from `rsx-dxs` so the transport stays
  domain-agnostic
- 11 `#[repr(C, align(64))]` records on top of `rsx-dxs`
- FillRecord, BboRecord, OrderInsertedRecord,
  OrderCancelledRecord, OrderDoneRecord,
  OrderAcceptedRecord, OrderFailedRecord,
  MarkPriceRecord, LiquidationRecord,
  ConfigAppliedRecord, CancelRequest
- Per-type encode/decode helpers
- 22 compile-time size+align asserts pin the wire layout
- New record types added without editing the transport

### rsx-gateway

- WS server on monoio with io_uring (gateway and marketdata
  only — matching/risk/mark/recorder run on tokio)
- JWT hardening: HS256, 32-byte minimum secret enforced at
  boot, `exp` + `nbf` validated, `JtiTracker` dormant
  (wired but disabled until reuse-detection is needed)
- Rate limiting: per-user, per-IP (FIFO eviction at the
  cap so the map is bounded), per-instance
- Circuit breaker (open/half-open/closed)
- Pending order tracking with timeout
- casting/UDP transport to Risk process
- Heartbeat timer, order dedup
- Stream ordering, pre-validation (tick/lot)
- REST contract endpoints
- Gateway mode endpoint (/v1/gateway/mode)

### rsx-risk

- User accounts: deposit, withdraw, balance
- Position tracking per symbol
- Portfolio margin: IMR/MMR calculation
- Liquidation engine: rounds, slip BPS
- Insurance fund per symbol
- Funding accumulation and settlement
- Advisory lease (main/replica promotion)
- WAL persistence, Postgres replay
- casting/UDP router, multi-symbol ME addressing
- Shard routing (user_id % N)
- SPSC rings per symbol (fan-out)
- Crash-restart with exponential backoff

### rsx-marketdata

- Shadow orderbook from INSERT/CANCEL events
- BBO derivation per symbol
- L2 depth (configurable, default 20 levels)
- Trade tape (200 entry cap)
- WS broadcast to subscribers
- Seq gap detection, snapshot bootstrap
- Heartbeat, per-symbol subscriptions
- Multi-ME casting subscription

### rsx-mark

- External sources: Binance WS, Coinbase WS
- Weighted mean aggregation
- Staleness filter, fallback logic
- WAL writer for MARK_PRICE records
- replication for consumers
- Periodic sweep (1s), reconnect with backoff

### rsx-recorder

- replication consumer (subscribes to WAL stream)
- Daily WAL rotation
- Buffered writes (flush every 1000 records)
- Graceful shutdown on SIGINT/SIGTERM

### rsx-cli

- `wal-dump <stream_id> <dir>`: dump WAL directory
- `dump <file>`: dump single WAL file
- JSON lines output, record type names
- Full payload decoding (all record types)
- Filters: --type, --symbol, --user, --from-ts, --to-ts
- --stats: aggregate counts by record type
- --follow: tail mode with ctrlc handler
- --tick-size, --lot-size: display scale

### rsx-maker

- WS client to Gateway
- Two-sided quoting: bid+ask ladder
- Order cancellation cycle
- Exponential backoff reconnect
- SIGINT/SIGTERM shutdown
- Env config: mid, spread_bps, levels, qty, tick,
  lot, refresh_ms

## Playground Dashboard

14 tabs: Overview, Topology, Book, Risk, WAL, Logs,
Control, Maker, Faults, Verify, Orders, Stress, Docs,
Trade.

60+ API endpoints:

- Process: start/stop/restart per process
- Orders: submit, cancel, batch
- Risk: deposit, freeze, liquidate, overview, funding,
  insurance fund
- WAL: dump, verify integrity
- Market data: book snapshot, BBO, mark price
- Maker: start/stop lifecycle
- Sessions: allocate, renew, release
- Stress: start/stop/status (subprocess management)
- v1 proxy: symbols, candles, funding, positions, fills
- Gateway mode: /v1/gateway/mode

## React WebUI (rsx-webui/)

Components: Orderbook, OrderEntry, OpenOrders, Positions,
Funding, OrderHistory, Assets, TradesTape, DepthChart,
Chart, TopBar, BottomTabs, TradeLayout, Toast,
ErrorBoundary.

Stores: market, trading, connection, settings.

Hooks: usePublicWs, usePrivateWs, useRestApi, useKeyboard,
useSoundAlerts.

## Spec Coverage

| Area           | Specs                                  |
|----------------|----------------------------------------|
| Architecture   | ARCHITECTURE, TILES, NETWORK, PROCESS  |
| Matching       | MATCHING, ORDERBOOK, CONSISTENCY       |
| Transport      | replication, WAL, casting                         |
| Risk           | RISK, LIQUIDATOR                       |
| Market data    | MARKETDATA, MARK                       |
| Gateway        | GATEWAY, MESSAGES, WEBPROTO, RPC, REST |
| Edge cases     | VALIDATION-EDGE-CASES, POSITION-EDGE   |
| Infrastructure | DATABASE, DEPLOY, TELEMETRY, ARCHIVE   |
| Dashboard      | DASHBOARD + 4 domain dashboards        |
| Testing        | TESTING + 10 TESTING-*.md files        |
| Operations     | SCENARIOS, SIM, CLI, TRADE-UI          |
| **Total**      | **50+ spec files in specs/2/**        |

## Test Coverage

| Suite      | Files | Count  | Time    |
|------------|-------|--------|---------|
| Rust unit  | 88    | 878 pass / 912 attrs | <5s     |
| Playwright | 23    | 421 / 424 (3 skips)  | ~60s    |
| Python     | 21    | ~930   | ~10s    |
| WAL        | -     | -      | <10s    |
| E2E        | -     | -      | ~30s    |
| Integration| -     | -      | 1-5min  |

## Build Targets

| Target          | Command           | What                   |
|-----------------|-------------------|------------------------|
| check           | `make check`      | cargo check            |
| lint            | `make lint`       | clippy                 |
| unit tests      | `make test`       | unit tests (<5s)       |
| WAL tests       | `make wal`        | WAL correctness        |
| E2E             | `make e2e`        | Rust E2E + Playwright  |
| integration     | `make integration`| testcontainers (PG)    |
| benchmarks      | `make perf`       | Criterion              |
| bench gate      | `make bench-gate` | regression gate (10%)  |
| Playwright      | `make play`       | 421 browser tests (canonical) |
| release gate    | `make gate`       | all 4 release gates    |
| CI              | `make ci`         | phases 1-3             |
| CI full         | `make ci-full`    | all phases + shard fan |

## Invariants (named in code)

All 10 invariants from `CLAUDE.md` "Correctness Invariants
(system-wide)" carry a `// INVARIANT N:` comment at the
enforcement site (fills-before-done, exactly-one
completion, FIFO per level, position = Σ fills, monotonic
tips, no crossed book, SPSC FIFO, slab no-leak, funding
zero-sum, advisory-lock exclusivity).

## Stats

- ~21k LOC Rust, ~25k LOC Python, ~5k LOC TypeScript
- 12 Rust crates (+ rsx-playground, rsx-webui, rsx-auth
  outside the cargo workspace), 8 binaries
- 50+ specs, 88 Rust test files, 22 Playwright specs
- ~28 `[refine]` commits + ~12 a16z-fixes commits since v0.1.0
