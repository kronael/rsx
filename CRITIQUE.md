# Critique (Current State)

This critique reflects the repo as it exists now. I did not run tests.
It is based on code+specs cross-check and the CODEPATHS map.

## Top Findings (Ordered by Severity)

### Critical

1) **Price inputs to risk are still brittle.**
   Mark now sends CMP, and risk ingests it, but there is no end‑to‑end
   test for mark→risk or ME BBO→risk. If either feed is down, margin
   checks use stale/zero prices with no fallback.
   - Code: `rsx-mark/src/main.rs`, `rsx-risk/src/main.rs`
   - Tests missing: no integration tests for mark/BBO feed.

2) **Frozen margin release relies on per‑order map with no replay path.**
   The new `frozen_orders` map is memory‑only; on restart, frozen margin
   remains in account state but per‑order entries are lost, so cancel/done
   after restart cannot release correctly.
   - Code: `rsx-risk/src/shard.rs` (`frozen_orders`)
   - Underspec: no persistence/replay for per‑order frozen tracking.

3) **OrderDone status mapping is underspecified.**
   Gateway maps `final_status` to WS `status`, but spec doesn’t define
   the mapping explicitly. This is now code-defined behavior.
   - Code: `rsx-gateway/src/main.rs`
   - Spec gap: `specs/v1/WEBPROTO.md` lacks mapping from `final_status`.

### High

4) **Cancel path still depends on gateway state.**
   Cancels by client‑id require `pending` lookup. There is no stateless
   cancel correlation on the wire (order_id only).
   - Code: `rsx-gateway/src/pending.rs`, `rsx-gateway/src/handler.rs`
   - Spec gap: no explicit statement that gateway must persist pending
     for cancel by `cid`.

5) **Marketdata backpressure semantics still weak.**
   Outbound queue drops silently; seq gaps trigger snapshots but do not
   inform clients of dropped deltas. Empty‑book subscribe still yields
   no snapshot.
   - Code: `rsx-marketdata/src/state.rs`, `rsx-marketdata/src/handler.rs`
   - Tests missing: empty‑book snapshot; drop/resubscribe policy.

### Medium

6) **Risk reject reason mapping is hard-coded to FailureReason.**
   This fixes client codes but conflates `UserInLiquidation` and
   `NotInShard` as `InternalError`. That mapping is now policy without
   spec backing.
   - Code: `rsx-risk/src/main.rs`
   - Spec gap: WEBPROTO lacks explicit mapping for risk‑side rejections.

7) **Matching fan‑out tests still model SPSC, not CMP.**
   `rsx-matching/tests/fanout_test.rs` validates in‑process SPSC fanout;
   it does not cover CMP payload encoding or network ordering.

## Component-by-Component Mismatch Review

### Gateway

- **Spec vs code:** Spec says no pre‑trade ack; code now complies. Good.
- **Remaining gaps:** Cancel by `cid` requires pending state; no stateless
  cancel. OrderDone status mapping not defined in spec.
- **Tests:** parsing+rate-limit+JWT exist; no e2e WS order lifecycle test.

### Risk

- **Spec vs code:** CMP between processes; replica exists via CMP tip sync.
- **Gaps:** per‑order frozen map not persisted; no replay linkage; mark/BBO
  integration untested; rejection mapping policy undefined.
- **Tests:** margin/position/replication tests exist; no mark/BBO ingestion tests.

### Matching

- **Spec vs code:** emits events + WAL + DXS. BBO is emitted but only
  tested via SPSC fanout tests.
- **Gaps:** no CMP‑level fanout tests; config polling not implemented.

### Marketdata

- **Spec vs code:** CMP ingest, replay bootstrap, seq‑gap detection. Good.
- **Gaps:** empty‑book snapshot missing; backpressure semantics underspecified.
- **Tests:** replay/seq‑gap/shadow tests exist; no empty‑book snapshot test.

### Mark

- **Spec vs code:** now sends CMP to risk + WAL/DXS. Good.
- **Gaps:** no tests for CMP feed or symbol_map correctness end‑to‑end.

### DXS/CMP/WAL

- **Spec vs code:** header+payload; flow control via bool return; fsync flush.
- **Gaps:** CMP retry/backpressure behavior not covered by tests.

## Underspecified Areas (Spec Gaps)

- Mapping from `OrderDone.final_status` to WS `OrderStatus`.
- Mapping from risk reject reasons to WEBPROTO failure codes.
- Behavior on mark/BBO feed loss (fallback/timeout policy).
- Cancel by client_id: requirement for gateway state or wire support.
- Marketdata behavior on empty‑book subscribe and delta drops.
- Persistence/replay model for per‑order frozen margin tracking.

## Test Coverage Gaps (by Codepath)

- Mark→Risk CMP integration.
- ME BBO→Risk integration.
- Cancel/done margin release through full CMP flow.
- End‑to‑end WS order lifecycle (NewOrder→ME→Fill/Done/Fail).
- Marketdata empty‑book snapshot and drop/resubscribe policy.
- CMP flow‑control/NAK retransmit behavior.

## Bottom Line

Major flows are implemented, but pricing feeds, margin release durability,
status/reason mapping policy, and marketdata backpressure remain under‑specified
and under‑tested. The system is close, but correctness still depends on
implicit behaviors not codified in specs or tests.
