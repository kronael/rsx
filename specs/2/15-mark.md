---
status: shipped
---

# Mark Price Aggregator

Standalone network service. Aggregates mark prices from external
exchange WebSocket feeds, publishes to risk engines via casting/UDP,
and also writes a WAL for replication/recorder consumers.

Single process. Replaces the per-shard Binance async task that was
embedded in each risk engine (RISK.md section 4).

## Table of Contents

- [1. Architecture](#1-architecture)
- [2. Data Structures](#2-data-structures)
- [3. Source Connectors](#3-source-connectors)
- [4. Aggregation Logic](#4-aggregation-logic)
- [5. Serving Subscribers](#5-serving-subscribers)
- [6. Main Loop](#6-main-loop)
- [7. Config](#7-config)
- [8. RISK.md Changes](#8-riskmd-changes)
- [9. Performance Targets](#9-performance-targets)
- [10. File Organization](#10-file-organization)

---

## 1. Architecture

```
Binance WS ──┐
              ├──[SPSC]──> Aggregation Loop ──> WalWriter ──> ReplicationService
Coinbase WS ──┘            (single thread)        |
                                                  └──> casting/UDP -> Risk
```

- Exchange WS connectors run as async tasks, push to aggregation
  loop via SPSC rings.
- Aggregation loop is single-threaded, computes median mark price
  per symbol.
- WalWriter appends `MarkPriceRecord` records.
- casting/UDP sends `MarkPriceRecord` to Risk.
- ReplicationService (from `rsx-cast`) broadcasts to replay consumers.
- Recorder archives mark price stream to daily files.

---

## 2. Data Structures

### WAL/casting wire format

```rust
#[repr(C, align(64))]
struct MarkPriceRecord {
  seq: u64,
  ts_ns: u64,
  symbol_id: u32,
  _pad0: u32,
  mark_price: i64,     // fixed-point, same scale as Price
  source_mask: u32,    // bitmask of contributing sources
  source_count: u32,   // number of non-stale sources
  max_source_lag_ns: u32, // planned (see Observation freshness) — 0 until emitted
  _pad1: [u8; 20],     // align to 64 bytes
}
```

This record is emitted to both WAL and casting/UDP.

### Observation freshness (planned)

A mark price is only as current as the exchange observations behind it: a
consumer needs to know not just *when the mark was computed* (`ts_ns`) but
*how far before that the inputs were observed*. `max_source_lag_ns` carries
that — `ts_ns − min(timestamp_ns)` over the contributing sources, i.e. the
worst-case observation age of any source in this mark ("this mark is based
on data observed no more than X ago"). It fits the existing `_pad1` (additive,
64-byte record size unchanged, `0` until the mark engine populates it).

Downstream use: the risk margin fallback chain (§4) can weight a mark by its
freshness rather than only its binary stale/non-stale flag, and the trade
terminal surfaces it as the mark's staleness in its telemetry — the mark's
analogue of the order-path and marketdata latency legs (`55-terminal.md`).

This is a spec extension only; the mark engine is not yet updated to emit it.

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

    // Single-CRC fan-out: WAL + casting/UDP from one Framed
    let framed = wal.prepare(MarkPriceRecord { ... });
    wal.append_framed(&framed);
    cast.send_framed(&framed);
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

The aggregator embeds a `ReplicationService` from `rsx-cast`.

- Single `stream_id` for the mark price stream.
- Recorder connects as a replication consumer for archival.
- Risk engines consume mark prices via casting/UDP, not replication.

See [replication.md](replication.md) sections 5-6 for replay and consumer
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

The main loop is single-threaded and **ergonomic, not busy-spin**:
it drains its input rings, sweeps staleness, flushes the WAL, then
sleeps ~250µs. Mark is off the critical path — mark prices tick on
external-feed cadence and feed margin/liquidation, which tolerate
second-scale latency — so it must not burn a core. Async tasks (WS
connectors) run on a separate tokio runtime in background threads.
(A prior dedicated-core busy-spin was reverted 2026-05-29: unpinned,
it floated onto a hot-path core and starved it → UDP RcvbufErrors.)

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

**After:** risk engine connects as a replication consumer to the mark
price aggregator (this service). Receives `MarkPriceEvent` records
via replication streaming. No Binance dependency in the risk crate.

`binance.rs` is removed from `crates/rsx-risk/src/`.

The `binance_ring` in the risk main loop (RISK.md "Main Loop
Pseudocode" step 3) becomes a replication consumer callback that writes
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
rsx-mark/src/
    main.rs        -- entrypoint, runtime setup, main loop
    aggregator.rs  -- aggregation, staleness sweep, median
    source.rs      -- PriceSource trait, BinanceSource,
                      CoinbaseSource, WS reconnect loop
    types.rs       -- MarkPriceEvent, SourcePrice,
                      SymbolMarkState, SymbolMap
    config.rs      -- env config parsing
    lib.rs         -- re-exports
```

No core pinning. Mark is not on the critical
GW→ME→GW path, so the aggregator loop runs as a
plain ergonomic thread (sleep ~250µs, no `core_affinity`).
It MUST NOT busy-spin: an unpinned spinner floats across cores
and starves a pinned hot-path consumer (gateway/risk/ME),
whose UDP socket then overflows → kernel RcvbufErrors → dropped
packets → FAULTED storm.
