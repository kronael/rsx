# Critique (Current State)

This critique reflects the repo as it exists now. I did not run tests.

## Top Findings (Ordered by Severity)

### Critical

1) **Marketdata topology still violates spec.**
   Spec calls for in-process SPSC fan-out from Matching; implementation
   uses CMP/UDP and does not support DXS/WAL replay. This breaks ordering
   guarantees and recovery expectations for marketdata.
   - Files: `rsx-marketdata/src/main.rs`, `specs/v1/CONSISTENCY.md`

2) **Risk does not release frozen margin on cancel/done events.**
   `process_order_done` exists, but `OrderDoneEvent.frozen_amount` is set
   to `0` and cancels do not trigger release at all. Margin can remain
   over-reserved indefinitely.
   - Files: `rsx-risk/src/main.rs`, `rsx-risk/src/shard.rs`

### High

3) **Gateway emits no client-visible response on rejected orders.**
   With ack removal, if Risk rejects an order before it reaches Matching,
   the client never receives a response (no matching event). This violates
   expected UX and can leave clients waiting until timeout.
   - Files: `rsx-risk/src/main.rs`, `rsx-gateway/src/main.rs`, `specs/v1/WEBPROTO.md`

4) **Marketdata WS flow is incomplete relative to spec.**
   No snapshot on subscribe when book is empty, no seq-gap detection, no
   backpressure resubscribe path. Deltas can be silently dropped.
   - Files: `rsx-marketdata/src/main.rs`, `rsx-marketdata/src/state.rs`,
     `specs/v1/TESTING-MARKETDATA.md`

### Medium

5) **Order correlation for gateway cancel by client ID is still stateful.**
   Gateway keeps a pending queue to resolve `cid -> order_id`, but no
   matching event carries `client_order_id`. This means correlation still
   depends on gateway state, contrary to the stated goal.
   - Files: `rsx-gateway/src/pending.rs`, `rsx-gateway/src/handler.rs`

## Verified Improvements

- Gateway now routes `OrderInserted`, `OrderCancelled`, `OrderDone`, and
  `Fill` events back to clients.
- Risk forwards ME events to Gateway and no longer sends pre-trade acks.
- Time utilities unified via `rsx-types/src/time.rs`.

## Test Reality

- Not run in this pass.

## Bottom Line

Order flow is closer to spec, but marketdata topology and risk margin
release are still correctness blockers. Gateway correlation for `cid`
still depends on local state, so stateless correlation has not been
achieved.
