# Market Data Service

Market data is served by a dedicated service. It consumes orderbook
events and exposes public marketdata over WebSocket (see WEBPROTO.md).

## Inputs

- CMP/UDP from Matching (orderbook events and BBO).
- WAL/TCP replay from Matching (bootstrap optional).

## Outputs

- WebSocket public feed (BBO, L2 snapshot, L2 delta, trades).
  Wire format is WEBPROTO.md.

## Notes

- `seq` is the matching engine event height. Monotonic per symbol.
- If a client falls behind, server may drop deltas and require
  re-subscription with a new snapshot.
