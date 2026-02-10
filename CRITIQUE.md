# Critique (Current State)

This critique reflects the repo as it exists now. I did not run tests.

## Top Findings (Ordered by Severity)

### Critical

1) **Risk margin release is still incorrect on cancel/done.**
   `OrderDoneEvent.frozen_amount` is set to 0 and cancels do not release
   frozen margin at all. This over-reserves margin indefinitely.
   - Files: `rsx-risk/src/main.rs`, `rsx-risk/src/shard.rs`

2) **Gateway has no client-visible rejection path when Risk rejects pre-trade.**
   With pre-trade acks removed, a rejected order never reaches Matching,
   so no event is emitted and the client gets no response.
   - Files: `rsx-risk/src/main.rs`, `rsx-gateway/src/main.rs`

### High

3) **Marketdata WS flow still incomplete.**
   No snapshot on subscribe when book is empty, no seq-gap detection,
   no resubscribe-on-backpressure. Deltas can be silently dropped.
   - Files: `rsx-marketdata/src/main.rs`, `rsx-marketdata/src/state.rs`

4) **Risk not wired to mark/BBO feeds.**
   `mark_prices` and `index_prices` remain zero unless manually updated;
   risk checks can be wrong under real prices.
   - Files: `rsx-risk/src/main.rs`, `rsx-risk/src/shard.rs`

### Medium

5) **Marketdata recovery via DXS replay is not implemented.**
   Specs now mark this as planned; code has no DXS consumer or replay.
   - Files: `rsx-marketdata/src/main.rs`, `specs/v1/MARKETDATA.md`

## Verified Improvements

- Gateway now routes `OrderInserted`, `OrderCancelled`, `OrderDone`, and
  `Fill` events to clients (no pre-trade ack).
- Risk forwards ME events to Gateway via CMP/UDP.
- Mark aggregator has connectors and a running main loop.
- Time utilities unified in `rsx-types/src/time.rs`.
- Specs updated to reflect CMP/UDP inter-process links.

## Test Reality

- Not run in this pass.

## Bottom Line

Order flow is closer to spec, but risk margin release and rejection
visibility are still correctness blockers. Marketdata still needs
snapshot/seq-gap/backpressure handling to be reliable.
