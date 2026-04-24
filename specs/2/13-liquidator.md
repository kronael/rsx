---
status: partial
---

# Liquidator Specification

## Table of Contents

- [Context](#context)
- [1. Liquidation State](#1-liquidation-state)
- [2. Backoff Schedule](#2-backoff-schedule)
- [3. Order Generation](#3-order-generation)
- [4. Lifecycle](#4-lifecycle)
- [5. Margin Recovery (Cancellation)](#5-margin-recovery-cancellation)
- [6. Frozen Margin Interaction](#6-frozen-margin-interaction)
- [7. Main Loop Integration](#7-main-loop-integration)
- [8. Persistence](#8-persistence)
- [9. Config](#9-config)
- [10. Edge Cases and Boundary Conditions](#10-edge-cases-and-boundary-conditions)
- [11. Performance Targets](#11-performance-targets)
- [12. File Organization](#12-file-organization)
- [13. Tests](#13-tests)

---

## Context

The liquidator is embedded in the risk engine (like the funding
engine in [RISK.md](RISK.md) section 5). Triggered by
`enqueue_liquidation()` from per-tick margin recalc
([RISK.md](RISK.md) section 7). Not a sidecar. It generates
reduce-only limit orders that chase the market with progressive
slippage and linear backoff.

## 1. Liquidation State

See `rsx-risk/src/liquidation.rs`

**Implementation note**: The current implementation tracks
liquidation state per (user_id, symbol_id) pair, NOT per user.
This means each symbol's position liquidates independently with
its own round counter and delay timer. A user with positions in
multiple symbols will have multiple `LiquidationState` entries.
This differs from the original spec's `pending_orders` approach
but is simpler and avoids coordinating rounds across symbols.

In-memory `Vec<LiquidationState>` on the shard. Small number
of concurrent liquidations expected (<100), so linear scan is
acceptable. If liquidation volume increases, migrate to
`FxHashMap<(u32, u32), LiquidationState>` keyed by
(user_id, symbol_id).

## 2. Backoff Schedule

Linear delay, quadratic slippage:

| Round | Delay | Slippage |
|-------|-------|----------|
| 1 | 1s | 1 bp |
| 2 | 2s | 4 bp |
| 3 | 3s | 9 bp |
| n | n * base_delay_ns | n^2 * base_slip_bps |

Configurable `base_delay_ns`, `base_slip_bps`, cap at
`max_slip_bps`. Continues until user is above maintenance
margin or all positions closed.

## 3. Order Generation

Per-position limit orders. Price determined by fallback chain:

1. **Mark price** (primary): from mark price aggregator
2. **Index price** (fallback): BBO-derived index from risk engine
3. **Last known mark price** (final fallback): most recent valid
   mark price cached by risk engine

The liquidator NEVER stalls waiting for price. If all sources
are unavailable, it uses the last known mark price (which may
be stale but prevents indefinite liquidation delay).

Each open position gets a closing order at
`price_source +/- round^2 * base_slip_bps`:

- Long position -> sell at `price - slippage`
- Short position -> buy at `price + slippage`

Order properties:
- `reduce_only = true` (NewOrder field 8, ME enforces)
- `is_liquidation = true` (RiskNewOrder field 11, risk skips
  margin check)
- ME clamps qty to position size via position tracking
- Routed to ME via same CMP/UDP link as normal orders

## 4. Lifecycle

See `rsx-risk/src/shard.rs` for the `enqueue_liquidation`,
`on_fill`, `on_order_done`, `on_order_failed`, and
`maybe_process_liquidations` implementations.

Key invariants:
- Already-liquidating users are not re-enqueued
- All pending non-liquidation orders are cancelled on entry (releases frozen margin)
- Margin is re-checked after cancellations; liquidation aborts if recovered
- On each round: cancel unfilled orders, re-check margin, escalate slippage
- `maybe_process_liquidations()` is called each main loop iteration (same pattern as `maybe_settle_funding`)

## 5. Margin Recovery (Cancellation)

Re-check margin at every opportunity:
- After each fill on a liquidation order
- At round escalation
- On fresh price tick (mark or BBO) for user in liquidation

If `equity >= maintenance_margin`: cancel all pending liquidation
orders, remove from liquidation map, set status=Cancelled.

## 6. Frozen Margin Interaction

- Cancel all user's pending non-liquidation orders on entering
  liquidation (releases frozen margin, may restore margin)
- Liquidation orders do NOT freeze margin (user is already
  underwater, no point reserving)
- Reject new non-liquidation orders from user while in
  liquidation (`order_while_user_being_liquidated_rejected`)

## 7. Main Loop Integration

Liquidation runs at step 1b in `Shard::run_once`, right
after fill draining and before order draining:

```
1.  Drain fills           (highest priority)
1b. Process liquidations  (positions are current)
2.  Drain orders
3.  Drain mark price updates
4.  Drain BBOs
5.  Funding settlement
```

Rationale: liquidation decisions depend on latest position
state. Drain fills first so positions reflect the most
recent activity; check liquidation eligibility before
accepting new orders that might further deteriorate margin.

See `rsx-risk/src/shard.rs::run_once`.

## 8. Persistence

Postgres table (append-only):

```sql
CREATE TABLE liquidation_events (
    user_id       INT NOT NULL,
    symbol_id     INT NOT NULL,
    round         INT NOT NULL,
    side          SMALLINT NOT NULL,
    price         BIGINT NOT NULL,
    qty           BIGINT NOT NULL,
    slippage_bps  INT NOT NULL,
    status        SMALLINT NOT NULL, -- 0=placed, 1=filled, 2=cancelled
    timestamp_ns  BIGINT NOT NULL,
    inserted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Written via same write-behind worker as positions/fills.

## 9. Config

```
RSX_LIQUIDATION_BASE_DELAY_NS=1000000000
RSX_LIQUIDATION_BASE_SLIP_BPS=1
RSX_LIQUIDATION_MAX_SLIP_BPS=500
RSX_LIQUIDATION_MAX_ROUNDS=50
```

**Post-max-rounds behavior:** After `max_rounds` is reached,
liquidation continues at `max_slip_bps` (100% slippage cap
from config) with no further delay between rounds. If the
position still cannot be closed (no counterparty at any
price), the remaining loss is socialized via the insurance
fund. Insurance fund deduction is logged as a
`liquidation_events` row with `status = 3` (socialized).

## 10. Edge Cases and Boundary Conditions

Full coverage in tests (see §13). Edge cases handled:

- **Round progression**: round 1 fires immediately (`last_order_ns=0`); subsequent rounds wait `N * base_delay_ns`; `round > max_rounds` emits `SocializedLoss` and marks Done
- **Slippage bounds**: `round^2 * base_slip_bps` capped at `max_slip_bps`; price underflow impossible within cap (500 bps max = 95% of mark)
- **Zero/unavailable mark price**: round skipped without incrementing counter; liquidation resumes when price returns; delay timer not reset (may catch up rapidly)
- **Zero position**: `maybe_process()` marks Done and skips order generation
- **Multiple symbols per user**: each (user_id, symbol_id) pair is an independent `LiquidationState`; rounds are not synchronized across symbols; margin recovery is per-symbol
- **Margin recovery**: risk engine calls `cancel_if_recovered` per symbol; liquidation engine is per-symbol only
- **Monotonic clock required**: non-monotonic `now_ns` stalls the delay check indefinitely; use `CLOCK_MONOTONIC`
- **Order rejected (symbol halted)**: liquidation for that symbol pauses; other failures treated as unfilled
- **Order fill race on round escalation**: benign — position updates first, new order uses updated position
- **Socialized loss**: `SocializedLoss` event emitted; insurance fund deduction is risk engine's responsibility; zero-mark-price at socialization is a data integrity issue (handle at consumer)
- **Config extremes**: `base_delay_ns=0` (immediate rounds), `max_rounds=0` (socialization after round 1), `max_slip_bps=0` (all orders at mark price)

## 11. Performance Targets

See liquidation benchmarks in `rsx-risk`. Targets: enqueue <100ns,
order generation <500ns/position, 100-user cascade <100us.

## 12. File Organization

```
crates/rsx-risk/src/
    liquidation.rs  -- LiquidationEngine, state, order gen
```

Added to existing risk crate, same as `funding.rs`.

## 13. Tests

Tests: see [TESTING-LIQUIDATOR.md](TESTING-LIQUIDATOR.md) for
complete unit tests, e2e tests, integration tests, benchmarks,
correctness invariants, and integration points.
