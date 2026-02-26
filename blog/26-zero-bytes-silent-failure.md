# Zero Bytes: Three Layers of Silent Failure

The mark price aggregator connects to Binance and Coinbase via
WebSocket, computes median prices, and writes them to WAL. After
deploying the playground with PENGU trading live (510KB WAL on
the matching engine), the mark WAL was 0 bytes. No errors in
the logs. No crashes. Just silence.

Fixing it required peeling back three layers, each invisible
without solving the previous one.

## Layer 1: No Sources Configured

The spawn plan set `RSX_MARK_SYMBOL_MAP=PENGU=10` but never set
`RSX_MARK_SOURCE_BINANCE_ENABLED=1` or the WS URL. Mark started
with `source_count=0` and logged exactly that — then sat in its
busy-spin loop with nothing to aggregate. Forever.

```
mark effective config: ... source_count=0
mark aggregator started
```

That's the entire log output. Two lines. No warning that zero
sources means zero data. The aggregator is correct — it has
nothing to aggregate — so it reports nothing wrong.

Fix: add the env vars to the spawn plan. Restart. Check the
logs again:

```
mark effective config: ... source_count=2
mark source name=binance ws_url=wss://stream.binance.com:9443/ws/penguusdt@trade
mark aggregator started
```

Two sources. WAL still 0 bytes after 60 seconds.

## Layer 2: TLS Not Compiled In

The WebSocket client loop in `source.rs` had this structure:

```rust
match connect_async(&ws_url).await {
    Ok((mut ws, _)) => {
        // read messages...
    }
    Err(_) => {
        consec_errors += 1;
    }
}
```

No logging on the `Err` branch. No logging on the `Ok` branch
either. The connection attempt, success, and failure were all
invisible.

Added `tracing::info!` on connect attempt and success,
`tracing::warn!` on failure. Rebuilt. The logs:

```
ws connecting to wss://stream.binance.com:9443/ws/penguusdt@trade
ws connect error: URL error: TLS support not compiled in
ws connecting to wss://stream.binance.com:9443/ws/penguusdt@trade
ws connect error: TLS support not compiled in
```

Repeating every few seconds with exponential backoff. The
`tokio-tungstenite` crate doesn't include TLS by default.
`wss://` URLs fail at the URL parsing stage — before any
network I/O — with an error that was being silently discarded
and counted as a generic connection failure.

The Cargo.toml had:

```toml
tokio-tungstenite = "0.24"
```

Fix:

```toml
tokio-tungstenite = { version = "0.24", features = ["rustls-tls-native-roots"] }
```

Rebuild. Now the logs show:

```
ws connecting to wss://stream.binance.com:9443/ws/penguusdt@trade
ws connected to wss://stream.binance.com:9443/ws/penguusdt@trade
```

Connected. WAL still 0 bytes.

## Layer 3: Symbol Name Mismatch

The Binance trade stream sends messages like:

```json
{"s": "PENGUUSDT", "p": "0.00687400", ...}
```

The symbol map had `PENGU=10`. The handler does:

```rust
let symbol_id = match symbol_map.get(symbol) {
    Some(id) => *id,
    None => return,  // silent discard
};
```

`symbol_map.get("PENGUUSDT")` returns `None` because the map
only has `"PENGU"`. Every single price update was silently
discarded. No log, no counter, no error.

Similarly, Coinbase sends `product_id: "PENGU-USD"` — also not
in the map.

Fix: the spawn plan now generates exchange-specific names for
each symbol:

```python
for s in symbols:
    mark_pairs.append(f"{name}={sid}")
    mark_pairs.append(f"{name}USDT={sid}")
    mark_pairs.append(f"{name}-USD={sid}")
```

And the Binance WS URL now includes the stream path directly:

```python
binance_ws_url = (
    f"wss://stream.binance.com:9443/ws/"
    f"{symbols[0]['name'].lower()}usdt@trade"
)
```

Because bare `wss://stream.binance.com:9443/ws` connects
successfully but sends nothing — Binance requires either a
stream-specific URL path or a subscribe message after connect.

Rebuild, restart. WAL goes from 0 to 720 bytes in 15 seconds.
1840 bytes after 30 seconds. Mark prices flowing.

## The Pattern

Three independent bugs, each masked by the previous:

| Layer | Bug | Symptom |
|-------|-----|---------|
| Config | No source env vars | `source_count=0`, no WS attempt |
| TLS | Feature flag missing | Connect fails, error swallowed |
| Names | `PENGU` vs `PENGUUSDT` | Prices discarded, no log |

Any one of these alone would have been found in minutes with
proper logging. Stacked together, they created 0 bytes of output
and 0 lines of diagnostic information. The system was
functioning correctly at every level — correctly doing nothing.

## What We Changed

1. **Always log connection lifecycle.** Connect attempt, success,
   and failure. The `Err(_) =>` branch with no body is a bug.

2. **Warn on zero sources at startup.** `source_count=0` should
   be a warning, not an info line buried in config output.

3. **Feature flags are load-bearing.** `tokio-tungstenite`
   without TLS parses `wss://` URLs and fails at URL validation,
   not at the network layer. The error message is accurate but
   only visible if you actually log it.

4. **Symbol maps cross system boundaries.** Internal names
   (`PENGU`) and exchange names (`PENGUUSDT`, `PENGU-USD`) are
   different strings. The mapping must happen somewhere explicit,
   not assumed to match.

The total fix was 6 lines of config, 1 line of Cargo.toml, and
15 lines of logging. The debugging took longer than the fix
because every layer was silent about its failure mode.
