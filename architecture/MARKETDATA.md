# Market Data Architecture

Shadow book reconstruction, public WebSocket broadcast.

## Data Flow

```
ME (CMP/UDP)        Marketdata           Users (WS)
+-----------+    +-----------------+    +--------+
| Inserts   |--->| CMP Receiver    |--->| BBO    |
| Cancels   |    |   |             |    | L2     |
| Fills     |    |   v             |    | Trades |
+-----------+    | Shadow Book     |    +--------+
                 |  (per symbol)   |
DXS Replay       |   |             |
+-----------+    |   v             |
| Bootstrap |--->| Subscription    |
| (startup) |    |  Manager        |
+-----------+    |   |             |
                 |   v             |
                 | Seq Gap Detect  |
                 |  -> L2 resend   |
                 +-----------------+
```

## Shadow Book

Per-symbol orderbook rebuilt from ME events:
- OrderInserted: add to book
- OrderCancelled: remove from book
- Fill: reduce qty, remove if zero

Produces BBO, L2 snapshots, L2 deltas, trades.

## Subscription Manager

Clients subscribe per-symbol per-channel:
- Channel 1: BBO updates
- Channel 2: L2 depth (snapshot + deltas)
- Channel 4: Trades

Snapshot sent on subscribe (if book non-empty).

## Seq Gap Detection

Tracks sequence numbers per symbol. On gap:
- Log warning
- Resend full L2 snapshot to depth subscribers

## DXS Replay Bootstrap

On startup, optionally replays ME WAL via DXS to
rebuild shadow book before accepting connections.

## Protocol

Same compact JSON as gateway (shared protocol crate):
- `BBO`: BBO update (9 fields)
- `B`: L2 snapshot (bids, asks arrays)
- `D`: L2 delta (single level change)
- `T`: Trade event

## Specs

- [specs/v1/MARKETDATA.md](../specs/v1/MARKETDATA.md)
- [specs/v1/TESTING-MARKETDATA.md](../specs/v1/TESTING-MARKETDATA.md)
