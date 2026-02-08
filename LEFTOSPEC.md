# Left to Specify

Gaps that block implementation, validated against existing specs.

## Must Have (blocks v1)

### Mark Price Aggregator

Currently only Binance mark price ingestion exists in `RISK.md`. There is no
spec for multi-source mark price aggregation, staleness handling, or fallback.

**Spec (TLDR):**
- Inputs: per-symbol price streams from N sources (e.g., Binance, Coinbase).
- Output: `mark_price[symbol_id]` updated on each tick.
- Method: median of last valid prices (ignore stale/outlier).
- Staleness: source stale if no update > 10s; drop from median.
- Fallback: if <2 sources active, use index price (BBO-derived).
- Publish: push `(symbol_id, mark_price, ts, source_mask)` to risk hot path (SPSC).

**Accuracy check:** This extends `RISK.md` (which only has Binance). If v1 stays
single-source, this section should be downgraded to Nice-to-Have or removed.

Touches: RISK.md (price feeds), CONSISTENCY.md (mark price stream), PROTOCOL.md (optional telemetry)

### Fees

Completely absent from all specs. No fee field in OrderFill proto,
no fee calc in risk engine, no fee column in fills table.

Needed:
- Fee config per symbol (maker_fee_bps, taker_fee_bps) in TOML
- Fee calc in risk engine on fill: `fee = qty * price * fee_rate`
- Deduct from user collateral on fill
- Add `fee` field to OrderFill in PROTOCOL.md
- Add `fee` column to `fills` table in RISK.md

Touches: PROTOCOL.md, RISK.md (small additions, not a new spec)

### Liquidation Order Generation

Detection is fully specified (RISK.md per-tick margin recalc,
`needs_liquidation`, `enqueue_liquidation`). Missing: what happens
inside `enqueue_liquidation`.

Needed:
- Sizing: close entire position or reduce to safe margin?
- Pricing: market order or aggressive limit at some spread?
- Priority: if 100 users need liquidation, largest shortfall first?
- Loss handling: what if liquidation fills at a loss? (insurance
  fund can be deferred, but the loss path must be defined)

Touches: RISK.md section 7 (20-line pseudocode addition)

## Nice to Have (not blocking, add when needed)

### Market Data (L2 Feed)

Raw event plumbing exists (CONSISTENCY.md: MktData gets Fill,
OrderInserted, OrderCancelled via SPSC ring). Missing: how
events become L2 snapshots/deltas for clients.

When needed, add to WEBPROTO.md:
- Snapshot message: `{B:[sym, bids, asks]}`
- Delta message: `{D:[sym, side, px, qty]}`
- Subscription protocol

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
- PROTOCOL.md: order flow, backpressure

Implementable as-is. No new spec needed.

### Admin/Config

TOML config + restart is sufficient for v1. Symbol config structs
defined in ORDERBOOK.md and RISK.md. Hot reload and runtime admin
API are v2 concerns.
