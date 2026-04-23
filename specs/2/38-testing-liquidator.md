---
status: shipped
---

# TESTING-LIQUIDATOR.md — Liquidation Engine Tests

Source spec: [LIQUIDATOR.md](LIQUIDATOR.md)

Module: `crates/rsx-risk/src/liquidation.rs`

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [Correctness Invariants](#correctness-invariants)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| L1 | Liquidation triggered when equity < maint_margin | §context |
| L2 | LiquidationState per user in FxHashMap | §1 |
| L3 | Linear delay: round * base_delay_ns | §2 |
| L4 | Quadratic slippage: round^2 * base_slip_bps | §2 |
| L5 | Slippage capped at max_slip_bps | §2 |
| L6 | Per-position limit orders at mark +/- slippage | §3 |
| L7 | Orders are reduce_only + is_liquidation | §3 |
| L8 | No re-enqueue if already in liquidation | §4 |
| L9 | Cancel non-liq orders on entry (release frozen) | §4, §6 |
| L10 | Re-check margin after cancel: may recover | §4 |
| L11 | Re-check margin after each fill | §4, §5 |
| L12 | Re-check margin at round escalation | §4 |
| L13 | Re-check on price tick for liquidating users | §5 |
| L14 | Cancel remaining orders if margin recovered | §5 |
| L15 | Liquidation orders skip margin check | §6 |
| L16 | Reject non-liq orders during liquidation | §6 |
| L17 | Liquidation orders do NOT freeze margin | §6 |
| L18 | Persistence: append-only liquidation_events table | §8 |
| L19 | Configurable base_delay, base_slip, max_slip | §9 |
| L20 | Max rounds configurable | §9 |
| L21 | Order failed (symbol halted): pause that symbol | §4 |
| L22 | Order failed (other): treat as unfilled, escalate | §4 |
| L23 | Status transitions: Active -> Cancelled or Completed | §1 |
| L24 | ME clamps qty to position size (reduce_only) | §3 |
| L25 | Orders routed via same CMP/UDP link as normal orders | §3 |
| L26 | Persisted via same write-behind worker as fills | §8 |
| L27 | First order fires immediately (last_order_ns=0) | §10.1 |
| L28 | Mark price=0 pauses round, no increment | §10.2 |
| L29 | Zero position during liq sets Done | §10.2 |
| L30 | Multiple symbols liquidate independently | §10.3 |
| L31 | Round timers per-symbol, not per-user | §10.3 |
| L32 | Monotonic clock assumed (no time backwards) | §10.4 |
| L33 | Rapid maybe_process calls safe (no dupe orders) | §10.4 |
| L34 | Slippage escalates even if orders filled | §10.5 |
| L35 | Socialized loss when round > max_rounds | §10.6 |
| L36 | base_delay_ns=0 fires all rounds immediately | §10.7 |
| L37 | max_rounds=0 allows round 1 then socializes | §10.7 |
| L38 | max_slip_bps caps prevent negative prices | §10.7 |

---

## Unit Tests

See `rsx-risk/tests/liquidation_test.rs` and
`rsx-risk/tests/margin_recalc_test.rs`.

---

## Correctness Invariants

1. **No re-enqueue** -- user in liquidation is never re-enqueued
2. **All positions get orders** -- every open position generates a
   closing order per round
3. **Margin re-check at every opportunity** -- fill, round, tick
4. **Recovery cancels all** -- if margin recovered, all pending
   liquidation orders cancelled
5. **No frozen margin on liquidation orders** -- user already
   underwater
6. **Non-liq orders rejected** -- while user is in liquidation
7. **Status terminal** -- Cancelled and Completed are terminal states,
   no further rounds placed
8. **Slippage monotonic** -- slippage never decreases across rounds
   (capped at max_slip_bps)
9. **Order count bounded** -- pending_orders.len() <= MAX_SYMBOLS
10. **First order immediate** -- round 1 fires on first maybe_process,
    no delay (last_order_ns=0 special case)
11. **Price non-negative** -- with max_slip_bps cap, liquidation
    prices never negative (sell >= 0, buy > 0)
12. **Round monotonic** -- round number only increases, never
    decreases or resets during Active status
13. **Zero position terminal** -- if position becomes zero during
    liquidation, status moves to Done, no further orders
14. **Mark price stall pauses, not fails** -- zero mark price pauses
    liquidation without incrementing round or marking failed
15. **Symbol independence** -- multiple symbols for same user liquidate
    independently, each with own round timer and state

---

## Integration Points

- Embedded in risk engine main loop (RISK.md §main loop step 5.5)
- Triggered by per-tick margin recalc (RISK.md §7)
- Generates reduce_only + is_liquidation orders to ME via same
  CMP/UDP link as normal orders (LIQUIDATOR.md §3)
- ME clamps qty to position size via position tracking
  (ORDERBOOK.md §6.5)
- Fills processed by normal fill path in risk engine (RISK.md §1)
- Cancels non-liquidation orders on entry, releasing frozen
  margin (RISK.md §6, LIQUIDATOR.md §6)
- Events persisted via risk write-behind worker (RISK.md §persistence)
- Gateway notified via Q frame on private WS (WEBPROTO.md §Q)
- System-level: liquidation cascade under price crash
  (TESTING.md §6 load tests)

Benchmark targets from LIQUIDATOR.md §10:

| Operation | Target |
|-----------|--------|
| Enqueue check | <100ns |
| Order generation per position | <500ns |
| Round escalation per user | <1us |
| 100-user cascade processing | <100us |
