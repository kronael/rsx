# Mark Price Aggregator

Standalone network service. Aggregates mark prices from external
exchange WebSocket feeds, publishes to risk engines via DXS
streaming ([DXS.md](DXS.md)).

Single process. Replaces the per-shard Binance async task that was
embedded in each risk engine (RISK.md section 4).

---

## 1. Architecture

```
Binance WS ──┐
              ├──[SPSC]──> Aggregation Loop ──> WalWriter
Coinbase WS ──┘            (single thread)        |
                                                   |
                                            DxsReplay server
                                           /       |
                                    Risk-0    Risk-1   Recorder
                                   (DXS consumer)
```

- Exchange WS connectors run as async tasks, push to aggregation
  loop via SPSC rings.
- Aggregation loop is single-threaded, computes median mark price
  per symbol.
- WalWriter appends `MarkPriceEvent` records.
- DxsReplay server (from `rsx-dxs`) broadcasts to subscribers.
- Risk engines connect as DXS consumers.
- Recorder archives mark price stream to daily files.

---

## 2. Data Structures

### WAL wire format

```rust
#[repr(C, align(64))]
struct MarkPriceEvent {
  symbol_id: u32,
  mark_price: i64,     // fixed-point, same scale as Price
  timestamp_ns: u64,
  source_mask: u32,    // bitmask of contributing sources
  source_count: u32,   // number of non-stale sources
  _pad: [u8; 12],      // align to 64 bytes
}
```

This record is one of the WAL event types streamed by DXS.

### In-memory

```rust
struct SourcePrice {
    source_id: u8,       // index into sources array
    price: i64,          // fixed-point
    timestamp_ns: u64,
}

struct SymbolMarkState {
    sources: [Option<SourcePrice>; 8],  // max 8 sources
    mark_price: i64,
    source_mask: u32,
    source_count: u8,
}
```

`SymbolMarkState` is indexed by `symbol_id` in a `Vec`.

---

## 3. Source Connectors

Each exchange feed implements the `PriceSource` trait:

```rust
trait PriceSource {
    /// Start the connector. Pushes SourcePrice updates to the
    /// provided SPSC producer. Handles reconnects internally.
    fn start(&self, tx: SpscProducer<SourcePrice>);
}
```

### BinanceSource

- Connects to Binance mark price WebSocket.
- Parses `markPrice` stream updates.
- Maps Binance symbol names to internal `symbol_id`.
- Pushes `SourcePrice` to aggregation loop via SPSC.

### CoinbaseSource (stub)

- Placeholder for second source.
- Same trait, same SPSC pattern.

### Reconnect

Backoff: 1s, 2s, 4s, 8s, capped at 30s. Reset on successful
message. Each connector runs as an async tokio task within the
same process.

---

## 4. Aggregation Logic

On each source update for a symbol:

```
fn aggregate(state: &mut SymbolMarkState, update: SourcePrice):
    state.sources[update.source_id] = Some(update)

    // Collect non-stale sources (stale = >10s since last update)
    let now = timestamp_ns()
    let fresh: Vec<i64> = state.sources.iter()
        .filter(|s| s.is_some())
        .filter(|s| now - s.timestamp_ns < STALENESS_NS)
        .map(|s| s.price)
        .collect()

    state.source_count = fresh.len()
    state.source_mask = compute_mask(state)

    match fresh.len() {
        0 => return,  // no publish; risk falls back to index
                      // price per RISK.md
        1 => state.mark_price = fresh[0],  // single source
        _ => {
            fresh.sort();
            state.mark_price = median(&fresh)
        }
    }

    // Append MarkPriceEvent to WAL (broadcasts to live consumers)
    wal.append(MarkPriceEvent {
        symbol_id, mark_price: state.mark_price,
        timestamp_ns: now, source_mask: state.source_mask,
        source_count: state.source_count as u32,
    })
```

**Staleness sweep:** every 1s, iterate all symbols. If a source
was non-stale and is now stale, re-aggregate and publish (the mark
price may change due to fewer sources).

**No publish on zero sources:** if all sources are stale, no
`MarkPriceEvent` is emitted. Risk engines fall back to index price
(size-weighted mid from BBO) per RISK.md section 4.

**Liquidation fallback:** If mark price is unavailable (all
sources stale), the liquidator uses this fallback chain:
mark -> index price (BBO) -> last known mark. See
LIQUIDATOR.md section 3 for the complete fallback chain.
The mark aggregator does NOT block or stall — it simply
stops publishing. Consumers handle the absence.

---

## 5. Serving Subscribers

The aggregator embeds a DxsReplay server from `rsx-dxs`.

- Single `stream_id` for the mark price stream.
- Risk engines connect as DXS consumers.
- On startup, risk engines replay from their last tip to catch up,
  then transition to live.
- Recorder connects as a DXS consumer for archival.

See [DXS.md](DXS.md) sections 5-6 for replay and consumer
protocol.

---

## 6. Main Loop

```
fn main_loop(sources: Vec<SpscConsumer<SourcePrice>>,
             wal: &mut WalWriter,
             states: &mut Vec<SymbolMarkState>):
    let mut last_sweep = timestamp_ns()
    loop {
        // 1. Drain source rings
        for ring in &sources:
            while let Ok(update) = ring.try_pop():
                aggregate(&mut states[update.symbol_id], update)

        // 2. Staleness sweep (every 1s)
        let now = timestamp_ns()
        if now - last_sweep > 1_000_000_000:
            for (sym_id, state) in states.iter_mut().enumerate():
                sweep_stale(sym_id, state, now, wal)
            last_sweep = now

        // 3. WalWriter flush (every 10ms, handled by wal.maybe_flush)
        wal.maybe_flush()
    }
```

The main loop is single-threaded, busy-spin. Async tasks (WS
connectors) run on a separate tokio runtime in background threads.

---

## 7. Config

```
RSX_MARK_LISTEN_ADDR=0.0.0.0:9200
RSX_MARK_WAL_DIR=./wal/mark
RSX_MARK_STREAM_ID=100
RSX_MARK_STALENESS_NS=10000000000

RSX_MARK_SOURCE_BINANCE_WS_URL=wss://fstream.binance.com/ws/!markPrice@arr@1s
RSX_MARK_SOURCE_BINANCE_ENABLED=1
RSX_MARK_SOURCE_BINANCE_RECONNECT_BASE_MS=1000
RSX_MARK_SOURCE_BINANCE_RECONNECT_MAX_MS=30000

RSX_MARK_SOURCE_COINBASE_WS_URL=wss://ws-feed.exchange.coinbase.com
RSX_MARK_SOURCE_COINBASE_ENABLED=0
RSX_MARK_SOURCE_COINBASE_RECONNECT_BASE_MS=1000
RSX_MARK_SOURCE_COINBASE_RECONNECT_MAX_MS=30000
```

---

## 8. RISK.md Changes

Section 4 "Price Feeds" subsection "From Binance" is replaced:

**Before:** async tokio task in risk process connects to Binance
mark price WS, pushes via SPSC ring.

**After:** risk engine connects as a DXS consumer to the mark
price aggregator (this service). Receives `MarkPriceEvent` records
via DXS streaming. No Binance dependency in the risk crate.

`binance.rs` is removed from `crates/rsx-risk/src/`.

The `binance_ring` in the risk main loop (RISK.md "Main Loop
Pseudocode" step 3) becomes a DXS consumer callback that writes
to the same SPSC ring, preserving the hot-path integration.

---

## 9. Performance Targets

| Path | Target |
|------|--------|
| Source to publish (end-to-end) | <100us |
| Publish to risk receipt (network) | <1ms |
| Aggregation per symbol | <500ns |
| Staleness sweep (100 symbols) | <50us |

---

## 10. File Organization

```
crates/rsx-mark/src/
    main.rs        -- entrypoint, config, runtime setup
    aggregator.rs  -- main loop, aggregation, staleness sweep
    source.rs      -- PriceSource trait, SPSC setup
    binance.rs     -- BinanceSource implementation
    types.rs       -- MarkPriceEvent, SourcePrice, SymbolMarkState
    config.rs      -- env config parsing
```
