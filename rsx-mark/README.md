# rsx-mark

Mark price aggregator binary. Feeds mark prices to Risk.

## What It Does

Connects to external exchange WebSocket feeds (Binance,
Coinbase), computes median mark prices per symbol, publishes
to Risk shards via CMP/UDP. Also writes to WAL for replay.

## Running

```
RSX_MARK_LISTEN_ADDR=127.0.0.1:9500 \
RSX_MARK_WAL_DIR=./tmp/wal \
RSX_MARK_STREAM_ID=mark \
RSX_RISK_MARK_CMP_ADDR=127.0.0.1:9400 \
RSX_MARK_STALENESS_NS=10000000000 \
RSX_MARK_PRICE_SCALE=100 \
cargo run -p rsx-mark
```

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_MARK_LISTEN_ADDR` | DXS replay listen address |
| `RSX_MARK_WAL_DIR` | WAL directory |
| `RSX_MARK_STREAM_ID` | WAL stream ID |
| `RSX_RISK_MARK_CMP_ADDR` | Risk CMP address for mark prices |
| `RSX_MARK_STALENESS_NS` | Source staleness threshold (10s) |
| `RSX_MARK_PRICE_SCALE` | Fixed-point price scale |

## Deployment

- Single instance (not sharded)
- Needs outbound internet for exchange WebSocket feeds
- Publishes to all Risk shards via CMP/UDP
- DXS replay sidecar serves historical mark prices

## Testing

```
cargo test -p rsx-mark
```

3 test files: aggregator, config, types.
See `specs/2/39-testing-mark.md`.

## Dependencies

- `rsx-types` -- shared types
- `rsx-dxs` -- WAL writer, CMP sender, DXS replay service
- tokio (for async WS source tasks)

## Gotchas

- Uses tokio for exchange WS feeds (async) but the
  aggregation loop is a single-threaded busy-spin. The two
  communicate via SPSC rings.
- If all sources go stale (>10s), no mark price is published.
  Risk falls back to index price (BBO).
- Staleness sweep runs every 1s. A source can be stale for
  up to 1s before detection.
- Price scale must match what Risk expects. Mismatch causes
  incorrect margin calculations.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- aggregation logic,
  staleness, source architecture, main loop
- `specs/2/15-mark.md`
