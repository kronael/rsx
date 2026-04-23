# rsx-marketdata Architecture

Market data process. Maintains shadow orderbooks from ME
events, publishes L2 depth, BBO, and trades to subscribed
WebSocket clients. See `specs/1/16-marketdata.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: monoio runtime, CMP pump, WS accept, seq gap detection |
| `shadow.rs` | `ShadowBook` -- shadow orderbook from ME events |
| `state.rs` | `MarketDataState` -- books, subscriptions, broadcasts |
| `subscription.rs` | Subscription management per client/symbol |
| `protocol.rs` | JSON serialization for L2, BBO, trades, snapshots |
| `handler.rs` | Per-connection: subscribe, unsubscribe |
| `ws.rs` | WebSocket accept loop on monoio |
| `config.rs` | `MarketDataConfig` from env vars |
| `types.rs` | Internal data structures |
| `replay.rs` | DXS replay bootstrap for book recovery |

## Key Types

- `MarketDataState` -- shadow books, subscriptions,
  connection registry
- `ShadowBook` -- per-symbol orderbook tracking
  (insert/cancel/fill by order ID)
- `Subscription` -- per-client channel subscriptions

## Data Flow

```
ME --[CMP/UDP]--> Marketdata
                    |
                 handler.rs
                    |
                 ShadowBook (rsx-book)
                    |
          +---------+---------+
          |         |         |
        L2 snap   BBO      Trades
          |         |         |
       WS clients (subscribed)
```

## CMP Decode Loop

1. Receive CMP/UDP datagram from ME
2. Decode WalHeader + payload
3. Dispatch by record type:
   - OrderInserted: insert into shadow book
   - OrderCancelled: remove from shadow book
   - Fill: update quantities, emit trade
   - BBO: update best bid/ask
   - ConfigApplied: update symbol config cache

## Publishing

| Feed | Content | Trigger |
|------|---------|---------|
| BBO | Best bid/ask price+qty | On best level change |
| L2 | Top N price levels | On any level change |
| Trades | Price, qty, side, ts | On each fill |

## Sequence Gap Detection

On gap detection:
1. Log warning with gap range
2. Automatically resend full L2 snapshot to depth subscribers
3. Shadow book may be inconsistent during gap

## Backpressure

Outbound WS queue per client. If a client falls behind:
- Deltas may be dropped silently
- Seq gap triggers automatic L2 snapshot resend

## Design Notes

- Single-threaded, dedicated core, busy-spin
- Separate process from gateway (public, no auth)
- One CMP/UDP input per matching engine
- No durable state (shadow book is ephemeral)
- DXS replay bootstrap on startup for book recovery
