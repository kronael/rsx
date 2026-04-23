---
status: shipped
---

# Architecture

Perpetuals exchange. Fixed-point arithmetic, single-threaded
matching per symbol, CMP/UDP between processes, WAL-based
recovery.

## Table of Contents

- [System Overview](#system-overview)
- [Crate Map](#crate-map)
- [Order Lifecycle](#order-lifecycle)
- [Hot Path](#hot-path)
- [Persistence and Recovery](#persistence-and-recovery)
- [Key Design Decisions](#key-design-decisions)
- [Spec Index](#spec-index)

---

## System Overview

```
                       External
                    +------------+
                    |  Web (WS)  |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS overlay + CMP bridge
                    | (monoio)   |  auth, rate limit, ingress bp
                    +-----+------+
                          | CMP/UDP
                    +-----v------+   CMP/UDP   +---------------+
                    |   Risk     +------------>| Matching Eng  |
                    |  Engine    |<------------+ (1 per symbol) |
                    | (1 shard)  |  CMP fills  +-------+-------+
                    +--+---+--+-+              |       |
                       |   |  |          +-----+  +----+-----+
              CMP/UDP  |   |  | CMP/UDP  |WAL     |CMP/UDP   |CMP/UDP
              +--------+   |  +------+   |        |          |
              v            |         v   v        v          v
         +--------+  +----+---+ +--------+  +---------+ +--------+
         |Postgres|  | Mark   | |Recorder|  |MARKETDATA| |Gateway |
         | (sync  |  | Price  | |(daily  |  |(shadow   | |(fills  |
         | commit)|  | Agg    | | WAL    |  | book,    | | back   |
         +--------+  |(Binance| | files) |  | L2/BBO)  | | to usr)|
                      | + N)  | +--------+  +---------+ +--------+
                      +-------+    DXS          DXS
```

Transports:
- **Between processes:** CMP/UDP for hot path; WAL/TCP for cold path
  (Gateway↔Risk↔ME). One record per datagram or TCP byte stream.
- **Within each process:** tiles (pinned threads) + SPSC
  rings (rtrb, 50-170ns) for internal handoff only.
- **DXS:** WAL streaming to consumers (recorder, marketdata).
  Transport is WAL/TCP on the cold path.

See `45-tiles.md` for tile pattern, `20-network.md` for process
topology.

## Crate Map

| Crate | Role |
|-------|------|
| rsx-book | Shared orderbook: PriceLevel, OrderSlot, Slab, CompressionMap |
| rsx-matching | ME tile logic, one instance per symbol, single-threaded |
| rsx-risk | Risk tile logic, one per user shard, margin + funding + liquidation |
| rsx-dxs | WAL writer/reader, DxsConsumer, DxsReplay server (transport-agnostic) |
| rsx-mark | Mark price aggregator (separate process), external WS feeds, median |
| rsx-gateway | Gateway tile, WS overlay + CMP bridge, auth, rate limit |
| rsx-marketdata | Marketdata tile, shadow book, L2/BBO/trades fan-out, public WS |
| rsx-recorder | Archival DXS consumer (separate process), daily WAL files |
| rsx-types | Price(i64), Qty(i64), Side, SymbolConfig newtypes |
| rsx-cli | WAL dump/inspect tool (clap CLI) |
| rsx-maker | Market maker bot (separate process) |

Non-Rust supporting projects (not in Cargo workspace):

| Project | Role |
|-------|------|
| rsx-playground | Dev dashboard (Python/FastAPI + Playwright tests) |
| rsx-webui | Trade UI SPA (TypeScript/React/Vite, built to dist/) |

Each process is a separate binary. Tile crates (rsx-book,
rsx-matching, rsx-risk, etc.) are libraries linked into
their respective process binaries.

## Order Lifecycle

```
User                Gateway          Risk           ME (BTC-PERP)
 |                    |                |                |
|--WS order--------->|                |                |
|                    |--CMP/UDP order>|                |
|                    |                |--margin chk--->|
|                    |                |--CMP/UDP order>|
|                    |                |                |--match book
|                    |                |                |--WAL append
|                    |                |<-CMP/UDP fills-|
|                    |                |--apply fill--->|
|                    |                |  position upd  |
|                    |                |--PG write-behind
|                    |<-CMP/UDP fills-|                |
|<--WS fill(s)-------|                |                |
|                    |                |                |
|                    |<-CMP/UDP done--|<-CMP/UDP done--|
|<--WS done----------|                |                |
```

Pre-trade: Risk checks portfolio margin (all positions, all
symbols for user) before routing to ME. Post-trade: Risk
applies fills, recalculates margin on every price tick,
triggers liquidation if equity < maintenance margin.

## Hot Path

Single-threaded ME per symbol. Dedicated pinned core.
Bare busy-spin on event loop (no spin_loop, no yield).

Key structures:
- OrderSlot: 128B, `#[repr(C, align(64))]`, hot fields in
  first cache line
- Slab arena: pre-allocated Vec + free list, O(1) alloc/free
- CompressionMap: distance-based zones (1:1 near mid, 1:1000
  far, catch-all at 50%+), ~617K slots = ~15MB per array
- Event buffer: fixed [Event; 10_000], reset via event_len=0

Zero heap allocation during matching. All prices/quantities
are i64 fixed-point (never float). Price-to-index via
bisection: 2-3 comparisons (~2-5ns).

Latency targets (same machine, intra-process SPSC):
- ME insert/match/cancel: 100-500ns
- SPSC hop: 50-170ns
- Risk fill processing: <1us
- Risk pre-trade check: <5us
- End-to-end order-to-fill: <50us

## Persistence and Recovery

```
  ME (per symbol)              Risk (per shard)
  +------------+               +------------------+
  | event buf  |               | in-memory state  |
  |  (10K fix) |               | positions,       |
  +-----+------+               | accounts, tips   |
        |                      +--------+---------+
        | drain                         |
  +-----v------+               +--------v---------+
  | WalWriter  |               | SPSC write-behind|
  | 10ms flush |               | ring (10ms flush)|
  | fsync      |               +--------+---------+
  +-----+------+                        |
        |                      +--------v---------+
  +-----v------+               |    Postgres      |
  | WAL files  |               | positions, fills |
  | 64MB rotate|               | tips, accounts   |
  | 10min retain               | sync_commit=on   |
  +-----+------+               +------------------+
        |
  +-----v------+     +------------------+
  | DxsReplay  |---->| Risk (consumer)  |
  | TCP server |     | replay tips+1    |
  +-----+------+     +------------------+
        |
  +-----v------+
  | Recorder   |  daily archive files
  | (DXS cons) |  infinite retention
  +------------+
```

WAL: 16B header + repr(C, align(64)) payload. Disk format =
wire format = stream format (no transformation). Flush every
10ms or 1000 records, fsync enforced. Backpressure: buffer
full or flush lag >10ms stalls the producer.

Recovery:
- ME: load snapshot, replay WAL from snapshot_seq+1
- Risk: load positions+tips from Postgres, DXS replay from
  tips[symbol]+1, go live on CaughtUp for all streams
- MARKETDATA: rebuild shadow book from ME WAL via DXS

Durability guarantees:
- Fills: 0ms loss (WAL, DXS replay, 10min retention)
- Orders in flight: can be lost (not WAL'd)
- Positions: 10ms loss (single crash), 100ms (dual crash)

## Key Design Decisions

- **Fixed-point i64**: deterministic arithmetic, no float
  rounding across architectures
- **CMP/UDP over broker**: direct UDP between processes,
  no Kafka/NATS; per-stream ordering and flow control
- **Slab arena**: pre-allocated 78M slots (~10GB), O(1)
  alloc/free, no malloc on hot path
- **Compressed indexing**: 5 distance-based zones reduce
  price level array from 20M to ~617K slots
- **Single-threaded per symbol**: no locks, no MESI
  invalidation, cache-line-aligned structs
- **Portfolio margin**: all positions across all symbols
  recalculated per price tick per exposed user
- **DXS brokerless streaming**: each producer IS the replay
  server; consumers connect directly, no central broker
- **Write-behind Postgres**: 10ms batched flush, COPY for
  fills, UPSERT for positions; backpressure at 100ms lag
- **Advisory locks**: Postgres pg_advisory_lock for
  single-writer per shard; auto-release on disconnect
- **SIGTERM = crash**: no graceful shutdown; one recovery
  path exercised on every restart
- **Incremental recentering**: copy-on-write array swap
  during mid-price drift, interleaved with matching

## Spec Index

See `specs/index.md` for the complete master index.
Key references:

| File | Covers |
|------|--------|
| 21-orderbook.md | Book data structures, matching algorithm, compression zones |
| 28-risk.md | Margin, positions, funding, liquidation triggers, main loop |
| 13-liquidator.md | Liquidation rounds, slippage, order generation |
| 10-dxs.md | WAL format, writer/reader, replay server, consumer |
| 48-wal.md | Shared WAL design, backpressure rules, flush bounds |
| 15-mark.md | Mark price aggregator, external feeds, median, staleness |
| 19-metadata.md | Symbol config scheduling, propagation, cold start |
| 6-consistency.md | Event fan-out, CMP/UDP routing, ordering guarantees |
| 20-network.md | Topology, transport, service discovery, startup ordering |
| 18-messages.md | Message semantics (transport is CMP/UDP) |
| 49-webproto.md | WS compact JSON protocol, frame types |
| 29-rpc.md | Async order handling, UUIDv7, pending tracking |
| 16-marketdata.md | Shadow book, L2/BBO/trades, public WS |
| 8-database.md | Postgres as system of record, write-behind pattern |
| 9-deploy.md | Single-machine topology, env config, ring sizing |
| 26-rest.md | REST API endpoints, request/response schemas |
| 33-telemetry.md | Structured metrics, tracing, log shipping |
| 44-testing.md | Test levels, make targets, invariants, benchmarks |
