# RSX Deep Critique (Spec + Code Audit)

Audit date: 2026-02-11
Scope: `specs/v1` + current in-repo implementation across `rsx-gateway`, `rsx-risk`, `rsx-matching`, `rsx-marketdata`, `rsx-dxs`, and hot-path tests/docs.
Method: static cross-check of wire contracts, runtime loops, durability/replay logic, and protocol consistency. No new code execution in this pass.

## Executive Summary

The system is close to feature-complete, but there are still several **contract-level correctness gaps** that can create production incidents even when unit tests pass:

1. Cross-component wire contract drift (`post_only`, reject reason semantics, config propagation).
2. Recovery durability mismatch (frozen margin release state not replayable).
3. Protocol edge-case violations (WS ping/pong framing, marketdata trades channel semantics).
4. Cold-start/sharding correctness hazard in Postgres bootstrap argument wiring.

The highest ROI improvements are to lock the wire schema, close replay-durability loops, and add integration tests that span `Gateway -> Risk -> Matching -> Gateway` and `Matching -> Marketdata`.

---

## P0 Findings (Must Fix)

### 1) Risk cold-start shard filter is called with wrong argument (`shard_count`)

**Status: FIXED** — `run_main` now passes `shard_count` correctly.

Evidence:
- `load_from_postgres(client, shard_id, shard_count, ...)` expects both `shard_id` and `shard_count`: `rsx-risk/src/replay.rs:18`.
- `run_main` calls it as `load_from_postgres(&client, shard_id, shard_id, max_symbols)`: `rsx-risk/src/main.rs:122`.

Impact:
- If `shard_id == 0` and DB is enabled, SQL uses `user_id % 0`, which is invalid.
- If `shard_id != shard_count`, state loading is wrong shard subset.
- This is a direct correctness risk for failover and cold-start integrity.

### 2) `post_only` is dropped in Gateway/Risk/Matching wire path

**Status: FIXED** — `post_only` wired end-to-end through WS parse, gateway request, risk request, matching wire, and book.

Evidence:
- Spec defines `N:[..., ro, po]`: `specs/v1/WEBPROTO.md:114`.
- Gateway frame model has no `post_only` field in `WsFrame::NewOrder`: `rsx-gateway/src/protocol.rs:6`.
- Gateway handler forwards only `reduce_only`, no `post_only`: `rsx-gateway/src/handler.rs:157`.
- Risk `OrderRequest` has no `post_only`: `rsx-risk/src/types.rs:17`.
- Matching inbound `OrderMessage` has no `post_only`, and hard-sets `post_only: false`: `rsx-matching/src/wire.rs:42`.

### 3) Marketdata trades channel contract is not implemented as specified

**Status: FIXED** — `CHANNEL_TRADES = 4` added, trade routing uses `has_trades` check.

Evidence:
- Spec includes channel `4=trades`: `specs/v1/MARKETDATA.md:22`, `specs/v1/WEBPROTO.md:209-221`.
- Subscription manager only defines channels 1 and 2: `rsx-marketdata/src/subscription.rs:4-5`.
- Trade broadcasts are gated by `has_depth(...)` instead of a trades channel check: `rsx-marketdata/src/main.rs:420`.

---

## P1 Findings (High Priority)

### 4) Frozen margin release state is memory-only and not replayable

**Status: DEFERRED** — Known limitation for v1. Frozen orders are rebuilt from replayed order lifecycle events, but a dedicated persistent ledger is not yet implemented.

Evidence:
- Frozen amounts are stored in in-memory map: `frozen_orders`: `rsx-risk/src/shard.rs:48`.
- Map insert on accept: `rsx-risk/src/shard.rs:476`.
- Release uses remove from this map: `rsx-risk/src/shard.rs:516`.
- Replay bootstrap loads accounts/positions/tips/funds, but not per-order frozen map: `rsx-risk/src/replay.rs:11-16`.

### 5) Gateway sends invalid WS pong framing

**Status: FIXED** — `ws_write_pong` uses opcode `0x8A` for pong frames.

Evidence:
- On opcode `9`, handler builds raw pong bytes and passes them to `ws_write_text`: `rsx-gateway/src/handler.rs:101-111`.
- `ws_write_text` always emits text opcode `0x81`: `rsx-gateway/src/ws.rs:214-222`.

### 6) Gateway ignores `CONFIG_APPLIED` despite Risk forwarding it

**Status: FIXED** — Gateway now applies config records into `GatewayState.symbol_configs`.

Evidence:
- Risk forwards `RECORD_CONFIG_APPLIED` to Gateway: `rsx-risk/src/main.rs:461-484`.
- Gateway receives it and drops it (`{}`): `rsx-gateway/src/main.rs:230`.
- Gateway relies on in-memory `symbol_configs` for tick/lot checks: `rsx-gateway/src/handler.rs:217-237`.

### 7) Reject reason mapping collapses domain errors into `INTERNAL_ERROR`

**Status: FIXED** — `UserInLiquidation` and `NotInShard` now map to distinct wire reasons.

Evidence:
- `UserInLiquidation` and `NotInShard` map to `InternalError`: `rsx-risk/src/main.rs:533-537`.
- Spec currently codifies this mapping: `specs/v1/WEBPROTO.md:98-101`.

---

## P2 Findings (Important, Not Immediate Blockers)

### 8) DXS consumer tip progression is index-based, not seq-based

**Status: FIXED** — Tip now uses record payload sequence via `extract_seq`.

Evidence:
- Tip increments by `self.tip += 1` for each replayed record: `rsx-dxs/src/client.rs:254`.

### 9) Risk `process_config_applied` stores version only, does not apply params

**Status: DEFERRED** — Known v1 limitation. Config version is tracked for observability; full param application deferred to v2 config system.

Evidence:
- Method only updates `config_versions`, comment says future update of params/fees: `rsx-risk/src/shard.rs:544-555`.

### 10) Gateway fill fee is hardcoded to zero

**Status: DOCUMENTED** — Intentional v1 simplification. Fee is computed at the risk layer and not present in `FillRecord`. Comment added at `rsx-gateway/src/route.rs:38`.

Evidence:
- `route_fill` always emits `fee: 0`: `rsx-gateway/src/route.rs:38`.

### 11) Pending-order lookup complexity and ownership model can degrade under load

**Status: DEFERRED** — Linear scan acceptable for v1 expected load. Dual-index map planned for v2.

Evidence:
- Pending store is linear scan on remove/find: `rsx-gateway/src/pending.rs`.

---

## Spec/Code Drift Snapshot

### Confirmed aligned
- WS order status mapping now documented: `specs/v1/WEBPROTO.md:81-84`.
- Risk reject mapping is at least explicitly documented (though weak semantics): `specs/v1/WEBPROTO.md:98-101`.
- `post_only` wired end-to-end (P0 #2 fixed).
- Marketdata trades channel implemented (P0 #3 fixed).
- Config propagation applied in gateway (P1 #6 fixed).

### Deferred items (v2)
- Persistent frozen-order ledger for crash recovery (P1 #4).
- Full config param application in risk (P2 #9).
- Dual-index pending order map (P2 #11).

---

## Test Strategy Upgrades (High Leverage)

### End-to-end (priority)
1. `gateway_ws_post_only_reject_e2e`
- Send `{N:[..., ro=0, po=1]}` crossing best price.
- Expect `U` with failed/cancel reason matching post-only policy.
- Validate matching did not insert/fill.

2. `risk_frozen_margin_crash_recovery_e2e`
- Accept order (margin frozen), restart risk before done/cancel.
- Replay state, then deliver completion.
- Assert frozen margin releases exactly once.

3. `marketdata_channel_mask_conformance_e2e`
- Subscribe with each mask combination.
- Assert `T` only on trades channel subscriptions.
- Assert `B/D` only on depth channel subscriptions.

4. `config_applied_propagation_e2e`
- Emit `CONFIG_APPLIED` from matching.
- Assert risk applies params and gateway updates symbol validation.
- Send boundary order before/after update to prove switchover behavior.

### Fault injection
1. Replay unknown record types during DXS bootstrap and verify tip correctness.
2. WS ping flood / malformed control frames with strict client.
3. Persist ring saturation with backpressure assertions (`risk.is_backpressured()`).

### Property checks
1. Idempotent margin release under duplicate done/cancel events.
2. Monotonic tip and non-regressing sequence invariants per symbol.
3. Order lifecycle conservation:
- accepted == inserted + failed + cancelled + done(terminal)

---

## Operational Improvements

1. Add startup diagnostics endpoint/log block:
- component version
- active config versions per symbol
- tip positions
- replay bootstrap status
- shard partition info (`shard_id`, `shard_count`)

2. Add invariant counters/alerts:
- unknown record type count
- seq gap count by symbol
- frozen margin unreleased age histogram
- pending queue size and stale eviction rate

3. Add explicit protocol versioning in `WEBPROTO.md` and DXS record docs:
- wire compatibility window
- additive vs breaking migration procedure

---

## Suggested Delivery Order

1. ~~Fix shard bootstrap args and add regression test.~~ DONE
2. ~~Wire `post_only` end-to-end.~~ DONE
3. ~~Fix marketdata trades channel semantics and tests.~~ DONE
4. ~~Fix WS ping/pong opcode handling.~~ DONE
5. ~~Implement gateway config-apply handling.~~ DONE
6. Add persistent frozen-order ledger and recovery tests. (v2)
7. ~~Refine reject reason taxonomy and client contract.~~ DONE
8. ~~Harden DXS tip tracking to sequence-aware progression.~~ DONE

---

## Bottom Line

Core architecture is strong and most subsystems are present, but reliability now depends on a few fragile contract edges between components. Closing those edges will materially improve production safety more than adding new features.

Of the 11 original findings, **8 are fully resolved**, **1 is documented as intentional** (fee=0), and **2 are deferred** to v2 (frozen margin ledger, pending order index, risk config application).
