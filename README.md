# RSX Exchange

Perpetuals exchange with fixed-point arithmetic,
single-threaded matching per symbol, and WAL-based recovery.

An end-to-end demonstration of high-frequency, low-latency
exchange design principles — from spec to working system.
Built with [Claude](https://claude.ai) and
[Codex](https://openai.com/codex), orchestrated by
[kronael/demiurg](https://github.com/kronael/demiurg).

## Status

Spec-first project. All specifications are in `specs/v1/`.
No implementation code yet.

## Architecture

Gateway accepts WS and gRPC connections, forwards orders
through SPSC rings to the risk engine, which validates margin
and routes to per-symbol matching engines. Fills flow back
through SPSC rings to risk (position updates) and gateway
(user notifications). WAL streaming (DXS) handles replay and
archival.

See [specs/v1/ARCHITECTURE.md](specs/v1/ARCHITECTURE.md).

## Components

- **Matching Engine** -- one per symbol, single-threaded,
  pinned core, GTC limit orders
- **Risk Engine** -- pre-trade margin, position tracking,
  funding, liquidation triggers
- **Gateway** -- WS overlay + gRPC passthrough, auth,
  rate limiting
- **Mark Price Aggregator** -- external exchange feeds,
  median price, staleness detection
- **Market Data** -- shadow book reconstruction, L2/BBO/trades
  fan-out over public WS
- **DXS** -- brokerless WAL streaming, each producer serves
  its own replay stream

## Crate Layout

```
crates/
  rsx-book/       orderbook (PriceLevel, OrderSlot, Slab)
  rsx-matching/   matching engine binary (one per symbol)
  rsx-risk/       risk engine binary (one per user shard)
  rsx-dxs/        WAL writer/reader, DxsReplay server
  rsx-mark/       mark price aggregator
  rsx-gateway/    WS overlay + gRPC passthrough
  rsx-marketdata/ market data fan-out (shadow book, L2/BBO)
  rsx-recorder/   archival consumer (daily WAL files)
  rsx-types/      Price, Qty, Side, SymbolConfig newtypes
```

## Build and Test

```
make test           # unit tests, <5s
make e2e            # component tests, ~30s
make integration    # testcontainers (Postgres), 1-5min
make wal            # WAL correctness, <10s
make smoke          # deployed system, <1min
make perf           # Criterion benchmarks, nightly
```

TOML config as first CLI argument, API keys as second.
No implementation yet -- these are target commands.

## Specs

Entry point: [specs/v1/ARCHITECTURE.md](specs/v1/ARCHITECTURE.md).

All specifications live in `specs/v1/`. Key files:

| Spec | Covers |
|------|--------|
| ORDERBOOK.md | Book structures, matching, compression |
| RISK.md | Margin, positions, funding, liquidation |
| DXS.md | WAL format, replay server, consumers |
| MARK.md | External feeds, median, staleness |
| CONSISTENCY.md | Event fan-out, ordering guarantees |
| DEPLOY.md | Topology, config, ring sizing |
| TESTING.md | Test levels, invariants, benchmarks |

## Design Principles

- **Fixed-point i64** -- deterministic arithmetic, no float
  rounding across architectures
- **Single-threaded per symbol** -- no locks, no cache
  invalidation, pinned cores
- **SPSC rings for IPC** -- rtrb, 50-170ns latency, no
  broker (Kafka/NATS)
- **WAL-based recovery** -- 0ms fill loss, idempotent replay
  from tip+1
- **Slab arena allocation** -- pre-allocated slots, zero heap
  on hot path
- **WAL = wire = stream** -- no format transformation between
  disk, network, and memory
