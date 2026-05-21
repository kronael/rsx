---
status: shipped
---

# Market Data Service

Market data is served by a dedicated service. It consumes orderbook
events and exposes public marketdata over WebSocket (see WEBPROTO.md).

## Inputs

- CMP/UDP from Matching (orderbook events and BBO).
- WAL/TCP replay from Matching (DXS replay bootstrap).

## Outputs

- WebSocket public feed (BBO, L2 snapshot, L2 delta, trades).
  Wire format is WEBPROTO.md.

## Subscribe Channels

| Channel | Name | Message | Description |
|---------|------|---------|-------------|
| 1 | bbo | `BBO` | Best bid/offer updates |
| 2 | depth | `B` (snapshot) + `D` (deltas) | L2 orderbook |
| 4 | trades | `T` | Individual trade executions |

Clients subscribe via `{S:[sym, channels]}` where `channels`
is a bitmask (e.g. 3 = bbo + depth, 7 = all three). See WEBPROTO.md.

## Notes

- `seq` is the matching engine event height. Monotonic per symbol.
- If a client falls behind, server may drop deltas and require
  re-subscription with a new snapshot.
- On outbound backpressure (per-client queue full), server
  clears that client queue and sends a fresh snapshot.
- Subscribing to a symbol sends a snapshot even if the book
  is empty (empty bids/asks arrays).

## Runtime Model

- Single-threaded monoio (io_uring) reactor. All state
  lives behind `Rc<RefCell<MarketDataState>>`; no locks,
  no cross-thread sharing.
- Shadow book reuses `rsx-book` `Orderbook` per symbol.
  Each ingress CMP record (`OrderInserted`, `OrderCancelled`,
  `Fill`) applies to the shadow book; deltas + BBO are
  derived and broadcast to subscribers of that symbol.
- L2 snapshot is sent on subscribe (channel bit 2 = depth)
  and on backpressure recovery; deltas thereafter.
- BBO (channel bit 1) and trades (channel bit 4) are
  independent subscriptions; trades are emitted from
  `Fill` events.
- Per-symbol shadow books are lazily allocated and evicted
  after a TTL with no subscribers.

## Multi-ME Aggregation

Marketdata listens to N matching engines concurrently
(one symbol → one ME). One `CmpReceiver` is bound per ME
on a derived local port. Records are demultiplexed by
`symbol_id` in the payload; each symbol's events flow
into its own shadow book independently.

## Seq Gaps and Cold Path

- Marketdata tracks an expected next `seq` per symbol on
  the CMP/UDP live path. A jump (`got > expected`) is a
  gap: marketdata broadcasts a fresh L2 snapshot to all
  depth subscribers for that symbol and resumes from the
  new `seq + 1`. Duplicates (`got < expected`) are ignored.
- On startup, marketdata may bootstrap via WAL/TCP replay
  (`DxsConsumer`) from a configured `replay_addr` and tip
  file. It replays `OrderInserted`, `OrderCancelled`, and
  `Fill` records into the shadow books until receiving
  `CaughtUp`, then switches to live CMP/UDP ingest. This
  is the only cold-path recovery; deeper gaps require
  client re-subscription.
