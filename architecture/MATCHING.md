# Matching Engine Architecture

One instance per symbol. Single-threaded, pinned core.

## Data Flow

```
Risk (CMP/UDP)          ME (per symbol)           Consumers
+-----------+     +-------------------------+     +--------+
| OrderReq  |---->| CMP Receiver            |     | Risk   |
+-----------+     |   |                     |     +--------+
                  |   v                     |         ^
                  | Dedup (5min window)     |    CMP/UDP
                  |   |                     |         |
                  |   v                     |     +--------+
                  | Orderbook               |---->| Mktdata|
                  |  Slab + CompressionMap  |     +--------+
                  |   |                     |
                  |   v                     |
                  | Event Buffer [10K]      |
                  |   |                     |
                  |   v                     |
                  | WAL Writer (10ms flush) |
                  |   |                     |
                  |   v                     |
                  | CMP Sender (fanout)     |
                  +-------------------------+
```

## Orderbook Structure

- **Slab arena**: pre-allocated Vec<OrderSlot> + free list
- **CompressionMap**: 5 distance-based zones reduce 20M
  price levels to ~617K slots (~15MB per side)
- **OrderSlot**: 128B, `#[repr(C, align(64))]`, hot fields
  in first cache line
- **Matching**: price-time FIFO within level

## Order Types

- GTC (Good-Til-Cancel): rests on book
- IOC (Immediate-Or-Cancel): fill or kill remainder
- FOK (Fill-Or-Kill): all or nothing
- Post-only: reject if would cross spread
- Reduce-only: only reduces existing position

## Event Fan-Out

ME emits events to two CMP senders:
1. **Risk**: fills, inserts, cancels, done, config_applied, BBO
2. **Marketdata**: fills, inserts, cancels

BBO emitted only when best bid/ask changes.
CONFIG_APPLIED emitted at startup and on config reload.

## WAL

- 16B header + repr(C) payload
- Flush: 10ms or 1000 records
- Rotate: 64MB, retain 10min
- DXS replay server for consumers

## Dedup

- OrderAcceptedRecord written to WAL on accept
- DedupTracker: FxHashMap<u128, u64> pruned every 5min
- Duplicate orders get OrderFailed response

## Recovery

1. Load snapshot (binary serialization)
2. Replay WAL from snapshot.last_seq + 1
3. Resume live processing

## Specs

- [specs/v1/ORDERBOOK.md](../specs/v1/ORDERBOOK.md)
- [specs/v1/MATCHING.md](../specs/v1/MATCHING.md)
- [specs/v1/CONSISTENCY.md](../specs/v1/CONSISTENCY.md)
