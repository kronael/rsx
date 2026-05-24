# rsx-mark Architecture

Mark price aggregator process. Consumes real-time prices from
exchange WebSocket feeds, computes median mark prices, publishes
to Risk via casting/UDP. See `specs/2/15-mark.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: source setup, aggregation loop, WAL + casting output |
| `aggregator.rs` | `aggregate()`, `median()`, `sweep_stale()`, `compute_mask()` |
| `source.rs` | `BinanceSource`, `CoinbaseSource`, WebSocket parsing |
| `types.rs` | `SourcePrice`, `SymbolMarkState`, `SymbolMap` |
| `config.rs` | `MarkConfig` from env vars |

## Key Types

- `SourcePrice` -- single price update: symbol_id, source_id,
  price, timestamp_ns
- `SymbolMarkState` -- per-symbol: up to 8 source slots,
  current mark price, source mask
- `BinanceSource` / `CoinbaseSource` -- WebSocket feed consumers
- `PriceSource` trait -- `fn start(tx: Producer<SourcePrice>)`

## Architecture Diagram

```
Binance WS --[async task]--> SPSC --> Aggregation Loop --> WalWriter
Coinbase WS -[async task]--> SPSC /   (single thread)       |
                                                        DxsReplay
                                                       /    |
                                                Risk-0  Risk-1  Recorder
```

Exchange WS connectors run as tokio async tasks. Push
SourcePrice updates to aggregation loop via SPSC rings.
Aggregation loop is single-threaded, busy-spin.

## Aggregation Logic

Per symbol, `SymbolMarkState` tracks up to 8 sources.
On each source update:
1. Store update in sources array
2. Filter stale sources (>10s since last update)
3. Compute mark price:
   - 0 fresh sources: no publish (risk uses index price)
   - 1 source: use directly
   - 2+ sources: median

## Staleness Sweep

Every 1s, iterate all symbols. If a previously-fresh source
became stale, re-aggregate and publish.

## Fallback Chain

If all sources stale, no MarkPriceEvent published. Risk
engine fallback: mark -> index price (BBO) -> last known mark.

## Main Loop

```
loop {
    1. Drain source SPSC rings
    2. Staleness sweep (every 1s)
    3. WalWriter flush (every 10ms)
    // busy-spin, no pause
}
```

## Performance Targets

| Path | Target |
|------|--------|
| Source to publish | <100us |
| Publish to risk receipt | <1ms |
| Aggregation per symbol | <500ns |
| Staleness sweep (100 symbols) | <50us |

## Architectural Decisions

**Runtime: tokio (with internal SPSC rings to a single-thread
aggregation loop).** The dominant work is external exchange
WebSocket feeds (Binance, Coinbase) — TLS, reconnect/backoff,
JSON parsing. tokio + `tokio-tungstenite` is the boring,
correct choice for that. Mark prices are downstream of order
flow, not on the GW→ME→GW critical path; latency budget is
milliseconds, not microseconds, so an async reactor pays its
way.

The aggregation loop itself is a single busy-spin thread fed
by one `rtrb` SPSC ring per source (drop-newest on full,
capacity 1024). That is the tile pattern in miniature; see
[`../notes/tiles.md`](../notes/tiles.md). No `core_affinity`
pinning — mark price aggregation does not warrant a dedicated
core when its consumers (risk shards) already have one each.
