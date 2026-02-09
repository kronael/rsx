# Left to Specify

Gaps that block implementation, validated against existing specs.

## Must Have (blocks v1)

### Mark Price Aggregator

Currently only Binance mark price ingestion exists in `RISK.md`. There is no
spec for multi-source mark price aggregation, staleness handling, or fallback.

**Spec (TLDR):**
- Inputs: per-symbol price streams from N sources (e.g., Binance, Coinbase).
- Output: `mark_price[symbol_id]` updated on each tick.
- Method: median of last valid prices (ignore stale/outliers).
- Staleness: source stale if no update > 10s; drop from median.
- Fallback: if <2 sources active, use index price (BBO-derived).
- Publish: push `(symbol_id, mark_price, ts, source_mask)` to risk hot path (SPSC).

**Accuracy check:** This extends `RISK.md` (which only has Binance). If v1 stays
single-source, this section should be downgraded to Nice-to-Have or removed.

Touches: RISK.md (price feeds), CONSISTENCY.md (mark price stream),
MESSAGES.md (optional telemetry)

### ~~Fees~~ DONE

Specified in MESSAGES.md (OrderFill fee fields), RISK.md (fee calc,
pre-trade fee reserve, fills table columns), WEBPROTO.md (F
message fee field). Maker rebates supported (negative maker_fee).

### ~~Liquidation Order Generation~~ DONE

Specified in LIQUIDATOR.md. Progressive reduce-only limit orders
with linear backoff and quadratic slippage.

## Nice to Have (not blocking, add when needed)

### ~~Market Data (L2 Feed)~~ DONE

Specified in MARKETDATA.md. Shadow orderbook per symbol via shared
`rsx-book` crate, L2 depth/BBO/trades, public WS endpoint,
snapshot-then-incremental protocol.

### Binance Feed Details

Architecture covered in RISK.md section 4. Missing minor details:
- Reconnect backoff (1s, 2s, 4s, 8s, max 30s)
- Staleness threshold (e.g. 10s no update = stale)
- Stale behavior (use last known, log warning)

10-line addition to RISK.md section 4.

## Already Covered (no action needed)

### Gateway

Auth, sessions, TLS, rate limiting all specified across:
- NETWORK.md: responsibilities, scaling, security boundary, TLS 1.3
- WEBPROTO.md: auth message, heartbeat
- RPC.md: session handling, token bucket rate limiting with numbers
- GRPC.md: order flow, backpressure

Implementable as-is. No new spec needed.

### Admin/Config

TOML config + restart is sufficient for v1. Symbol config structs
defined in ORDERBOOK.md and RISK.md. Hot reload and runtime admin
API are v2 concerns.
