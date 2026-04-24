# FEATURES.md

Feature inventory for the RSX perpetuals exchange.

## System Overview

Spec-first perpetuals exchange with separate processes
communicating via CMP (C structs over UDP) and WAL
replication (TCP). Target: <50us GW->ME->GW round trip.

## Process Architecture

| Process   | Port | Role                                    |
|-----------|------|-----------------------------------------|
| Gateway   | 8080 | WS ingress, JWT auth, rate limit        |
| Risk      | UDP  | Per-shard margin, liquidation, funding  |
| Matching  | SPSC | Per-symbol, slab alloc, price-time FIFO |
| Marketdata| 8180 | Shadow book, L2/BBO/trades broadcast    |
| Mark      | 9201 | Binance/Coinbase aggregation, staleness |
| Recorder  | DXS  | Archival consumer, daily rotation       |
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
- Message encoding for outbound CMP

### rsx-dxs

- WalWriter: 10ms flush, 64MB rotate, 10min retain
- WalReader with sequence extraction
- DxsReplayService (TCP, from seq N)
- CMP protocol: UDP flow control, NACK-based
- 15 record types: FILL, BBO, ORDER_ACCEPTED,
  ORDER_DONE, ORDER_FAILED, ORDER_CANCEL_ACCEPTED,
  ORDER_CANCEL_REJECTED, MARK_PRICE, LIQUIDATION,
  CONFIG_APPLIED, CAUGHT_UP, and more
- TLS support, backpressure, tip persistence

### rsx-gateway

- WS server on monoio (io_uring, not epoll)
- JWT authentication
- Rate limiting: per-user, per-IP, per-instance
- Circuit breaker (open/half-open/closed)
- Pending order tracking with timeout
- CMP/UDP transport to Risk process
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
- CMP/UDP router, multi-symbol ME addressing
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
- Multi-ME CMP subscription

### rsx-mark

- External sources: Binance WS, Coinbase WS
- Weighted mean aggregation
- Staleness filter, fallback logic
- WAL writer for MARK_PRICE records
- DXS replay for consumers
- Periodic sweep (1s), reconnect with backoff

### rsx-recorder

- DXS consumer (subscribes to WAL stream)
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
| Transport      | DXS, WAL, CMP                         |
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
| Rust unit  | 88    | ~895   | <5s     |
| Playwright | 23    | 421    | ~60s    |
| Python     | 21    | 1048   | ~10s    |
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
| Playwright      | `make play`       | all 421 browser tests  |
| release gate    | `make gate`       | all 4 release gates    |
| CI              | `make ci`         | phases 1-3             |
| CI full         | `make ci-full`    | all phases + shard fan |

## Stats

- ~21k LOC Rust, ~25k LOC Python, ~5k LOC TypeScript
- 11 crates, 8 binaries
- 50+ specs, 88 Rust test files, 22 Playwright specs
