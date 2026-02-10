# Critique (Current State)

This critique reflects the repo as it exists now. I did not run tests.

## Top Findings (Ordered by Severity)

### Critical

1) **Risk is not wired to mark or BBO feeds, so margin checks use zero prices.**
   `mark_prices` and `index_prices` stay at 0 because nothing feeds the
   `MarkPriceUpdate` or `BboUpdate` rings. This makes liquidation checks,
   margin requirements, and funding calculations incorrect.
   - Files: `rsx-risk/src/main.rs`, `rsx-risk/src/shard.rs`

2) **Order lifecycle events are not consumed by Risk, so frozen margin is never released.**
   Risk only ingests fills; it ignores `OrderCancelled/OrderDone` from ME.
   `process_order_done` exists but is never called. This will over-reserve
   margin indefinitely.
   - Files: `rsx-risk/src/main.rs`, `rsx-risk/src/shard.rs`, `rsx-matching/src/main.rs`

### High

3) **Marketdata process is still a stub.**
   CMP receive loop discards payloads, shadow book not updated, WS broadcast
   loop is TODO. No live marketdata dissemination.
   - Files: `rsx-marketdata/src/main.rs`

4) **Gateway cannot correlate accepts/rejects to the client order.**
   `OrderResponse` lacks `order_id`; gateway pending queue is unused; response
   messages are sent with empty `order_id`. This breaks client reconciliation.
   - Files: `rsx-risk/src/rings.rs`, `rsx-gateway/src/handler.rs`, `rsx-gateway/src/main.rs`

### Medium

5) **Mark aggregator crate likely does not compile due to missing deps.**
   `rsx-mark` now uses tokio/tungstenite/serde_json but `Cargo.toml` does not
   include those deps. Build/test will fail until added.
   - Files: `rsx-mark/Cargo.toml`, `rsx-mark/src/source.rs`

## Verified Improvements

- Gateway → Risk → Matching wiring is present; orders flow into ME and fills
  flow back to Risk.
- Matching writes events to WAL and sends fills over CMP.
- CMP/WAL format is aligned (WalHeader + CmpRecord), no payload preamble drift.

## Test Reality

- Not run in this pass.

## Bottom Line

Core flow is closer, but risk correctness still hinges on missing price feeds
and order lifecycle accounting. Marketdata and gateway response correlation are
next blockers for a usable system.
