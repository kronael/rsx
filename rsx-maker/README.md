# rsx-maker

Market maker bot for RSX. Two-sided quoting through
gateway WebSocket.

## What It Does

Connects to gateway WS, places bid+ask ladders around a
configured mid price, cancels and replaces on a timer.
Separate process — managed by playground or run standalone.

## Running

```bash
cargo run -p rsx-maker
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| RSX_GW_WS_ADDR | ws://localhost:8080 | Gateway WS endpoint |
| RSX_MAKER_USER_ID | 99 | User ID for maker orders |
| RSX_MAKER_SYMBOL | 10 | Symbol ID to quote |
| RSX_MAKER_MID | 50000 | Mid price (raw ticks) |
| RSX_MAKER_SPREAD | 10 | Spread in basis points |
| RSX_MAKER_LEVELS | 5 | Levels per side |
| RSX_MAKER_QTY | 1000000 | Quantity per level (raw lots) |
| RSX_MAKER_TICK | 1 | Tick size (raw) |
| RSX_MAKER_LOT | 100000 | Lot size (raw) |
| RSX_MAKER_REFRESH | 2000 | Quote refresh interval (ms) |

## Architecture

```
rsx-maker ──WS──> Gateway ──CMP/UDP──> Risk ──> ME
```

Not on the critical path. Runs alongside exchange processes
as a load source for development and testing. Playground
manages its lifecycle via the Control tab.

## Behavior

1. Connect to gateway WS
2. Place `levels` bids and `levels` asks around `mid`
3. Wait `refresh_ms`, cancel all, repeat
4. On disconnect: exponential backoff reconnect (1s-30s)
5. SIGINT/SIGTERM: cancel outstanding, exit

## See Also

- [rsx-gateway](../rsx-gateway/README.md) — WS endpoint
- [rsx-playground](../rsx-playground/README.md) — manages maker lifecycle
- [specs/2/49-webproto.md](../specs/2/49-webproto.md) — WS frame format
