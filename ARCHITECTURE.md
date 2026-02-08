# Architecture

Perpetuals exchange. Fixed-point arithmetic, single-threaded
matching per symbol, SPSC rings for IPC, WAL-based recovery.

## System Overview

```
                       External
                    +------------+
                    |  Web (WS)  |
                    |  gRPC      |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS overlay + gRPC passthrough
                    |  (Tokio)   |  auth, rate limit, ingress bp
                    +-----+------+
                          | SPSC
                    +-----v------+     SPSC    +---------------+
                    |   Risk     +------------>| Matching Eng  |
                    |  Engine    |<------------+ (1 per symbol) |
                    | (1 shard)  |  SPSC fills +-------+-------+
                    +--+---+--+-+              |       |
                       |   |  |          +-----+  +----+-----+
                 gRPC  |   |  | SPSC     |WAL     |SPSC      |SPSC
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

Transports: SPSC rings (rtrb) for same-host hot path.
gRPC for cross-host, replay, and cold paths. DXS (WAL
streaming) for replay and archival consumers.

## Crate Map

| Crate | Role |
|-------|------|
| rsx-book | Shared orderbook: PriceLevel, OrderSlot, Slab, CompressionMap |
| rsx-matching | ME binary, one per symbol, single-threaded |
| rsx-risk | Risk binary, one per user shard, margin + funding + liquidation |
| rsx-dxs | WAL writer/reader, DxsConsumer, DxsReplay gRPC server |
| rsx-mark | Mark price aggregator, external WS feeds, median |
| rsx-gateway | WS overlay + gRPC passthrough, auth, rate limit |
| rsx-marketdata | Shadow book, L2/BBO/trades fan-out, public WS |
| rsx-recorder | Archival DXS consumer, daily WAL files |
| rsx-types | Price(i64), Qty(i64), Side, SymbolConfig newtypes |

## Order Lifecycle

```
User                Gateway          Risk           ME (BTC-PERP)
 |                    |                |                |
 |--WS/gRPC order---->|                |                |
 |                    |--SPSC order--->|                |
 |                    |                |--margin chk--->|
 |                    |                |--SPSC order--->|
 |                    |                |                |--match book
 |                    |                |                |--WAL append
 |                    |                |<-SPSC fill(s)--|
 |                    |                |--apply fill--->|
 |                    |                |  position upd  |
 |                    |                |--PG write-behind
 |                    |<-SPSC fill(s)--|                |
 |<--WS/gRPC fill(s)--|                |                |
 |                    |                |                |
 |                    |<-SPSC done-----|<-SPSC done-----|
 |<--WS/gRPC done-----|                |                |
```

Pre-trade: Risk checks portfolio margin (all positions, all
symbols for user) before routing to ME. Post-trade: Risk
applies fills, recalculates margin on every price tick,
triggers liquidation if equity < maintenance margin.

## Hot Path

Single-threaded ME per symbol. Dedicated pinned core.
Bare busy-spin on SPSC rings (no spin_loop, no yield).

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

Latency targets (same machine, SPSC):
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
  | gRPC server|     | replay tips+1    |
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
- **SPSC over broker**: rtrb rings for IPC, no Kafka/NATS;
  50-170ns latency, producer stalls on full ring
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

| File | Covers |
|------|--------|
| ORDERBOOK.md | Book data structures, matching algorithm, compression zones |
| RISK.md | Margin, positions, funding, liquidation triggers, main loop |
| LIQUIDATOR.md | Liquidation rounds, slippage, order generation |
| DXS.md | WAL format, writer/reader, replay server, consumer |
| WAL.md | Shared WAL design, backpressure rules, flush bounds |
| MARK.md | Mark price aggregator, external feeds, median, staleness |
| METADATA.md | Symbol config scheduling, propagation, cold start |
| CONSISTENCY.md | Event fan-out, SPSC routing, ordering guarantees |
| NETWORK.md | Topology, transport, service discovery, startup ordering |
| GRPC.md | Protobuf schema, order states, deduplication |
| WEBPROTO.md | WS compact JSON protocol, frame types |
| RPC.md | Async order handling, UUIDv7, pending tracking |
| MARKETDATA.md | Shadow book, L2/BBO/trades, public WS/gRPC |
| DATABASE.md | Postgres as system of record, write-behind pattern |
| DEPLOY.md | Single-machine topology, env config, ring sizing |
| ARCHIVE.md | WAL offload, infinite retention, gap handling |
| TESTING.md | Test levels, make targets, invariants, benchmarks |