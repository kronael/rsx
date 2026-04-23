---
status: shipped
---

# Rust Market Maker

## Goal

Implement `rsx-maker` as a working market maker that connects to the
gateway via WebSocket (WEBPROTO), places two-sided limit quotes around
a configurable mid price, and cancels/replaces them on each refresh
cycle. Reacts to fill frames to update position tracking.

## File

`rsx-maker/src/main.rs` — replace the stub. May add
`rsx-maker/src/maker.rs` for the core logic.

## References

Read these files before implementing:
- `specs/1/49-webproto.md` — WS frame format (N, C, F, U, E, BBO)
- `rsx-playground/market_maker.py` — Python reference implementation
- `rsx-gateway/src/ws.rs` — handshake and frame format
- `rsx-types/src/` — Price, Qty, Side types

## Configuration (env vars)

```
RSX_GW_WS_ADDR    ws://localhost:8080  gateway WS address
RSX_MAKER_USER_ID 99                  authenticated user id
RSX_MAKER_SYMBOL  10                  symbol_id to quote
RSX_MAKER_MID     50000               mid price (raw units)
RSX_MAKER_SPREAD  10                  spread in bps (each side)
RSX_MAKER_LEVELS  5                   price levels each side
RSX_MAKER_QTY     1000000             qty per level (raw units,
                                      must be lot-aligned)
RSX_MAKER_TICK    1                   tick size (raw units)
RSX_MAKER_LOT     100000              lot size (raw units)
RSX_MAKER_REFRESH 2000                refresh interval ms
```

## Wire Protocol

Connect to `RSX_GW_WS_ADDR` with HTTP header `x-user-id: {user_id}`.

Frame format (JSON text frames, same as WEBPROTO):

**New order:**
```json
{"N": [symbol_id, side, px, qty, client_order_id, 0]}
```
- `side`: 0=bid, 1=ask
- `client_order_id`: string, unique per order (e.g. `"m00001"`)

**Cancel:**
```json
{"C": [client_order_id]}
```

**Gateway responses:**
- `{"A": {"cid": "...", "oid": N}}` — order accepted
- `{"F": {"oid": N, "qty": N, "px": N, "side": N}}` — fill
- `{"U": {"cid": "...", "status": "DONE"}}` — order done
- `{"E": {"code": N, "message": "..."}}` — error

## Implementation

### Main loop (tokio async)

```
connect WS with x-user-id header
loop every RSX_MAKER_REFRESH ms:
    1. send cancel for each active_cid
    2. drain responses (100ms timeout each), evict filled/done cids
    3. compute quotes:
         for level 0..N:
           bid_px = (mid - spread - level*step) / tick * tick
           ask_px = (mid + spread + level*step + tick - 1) / tick * tick
    4. send N bid orders + N ask orders
    5. drain 2 responses per order pair (200ms timeout)
    6. on fill/done frame: evict that cid from active_cids
```

### WS handshake

The gateway does the HTTP→WS upgrade. Use `tokio-tungstenite` or
`tungstenite` for the WS client. Send text frames (opcode 1).

Check `rsx-maker/Cargo.toml` for existing dependencies before adding.
Prefer `tungstenite` (sync) if tokio is not already present to keep
the binary small.

If `tokio` is already a dependency, use `tokio-tungstenite`.

### Cid generation

```rust
format!("m{:019}", counter)  // 20-char string
```

## Acceptance Criteria

1. `cargo build -p rsx-maker` — zero errors, zero unused warnings.
2. With gateway running: `RSX_MAKER_SYMBOL=10 RSX_MAKER_USER_ID=99 \
   RSX_MAKER_MID=50000 ./target/debug/rsx-maker` starts without
   panicking and logs "rsx-maker started" within 1s.
3. After 5s, the maker has placed at least 5 orders (observable via
   gateway logs showing connection + order frames).
4. After 10s, the maker has sent at least one cycle of cancels +
   new orders (observable from gateway log patterns).
5. Ctrl-C shuts down cleanly (no panic, no zombie threads).

## Constraints

- Connect to GATEWAY via WS (not directly to ME via CMP — that
  bypasses risk).
- All prices must be tick-aligned, all qtys must be lot-aligned.
- No heap allocation in the quote loop after initial setup.
- 80 char line width, max 120.
- Single import per line.
- Entrypoint called `main`.
