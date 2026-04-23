---
status: shipped
---

# Plan: 80% Completion — Close All Major Gaps

## Context

Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP)
Goal: Get all crates to 80%+ spec compliance. Main gap is
rsx-risk at 54% (missing liquidation, margin recalc, failover
basics). Gateway at 79% (missing ORDER_FAILED, heartbeats).

Approach: Simple, dumb code. No over-engineering. Tests for
every edge case. Document incomplete test coverage in
TESTING-*.md specs.

---

### Stage 1: Liquidation Engine Core

**Goal**: Implement basic liquidation engine in rsx-risk per
LIQUIDATOR.md. Enqueue, escalation rounds, order generation.
**Files**: rsx-risk/src/liquidation.rs (new), rsx-risk/src/shard.rs,
rsx-risk/src/lib.rs, rsx-risk/src/main.rs, rsx-risk/src/types.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-risk passes
- [ ] cargo test -p rsx-risk -- --skip persist --skip shard_e2e passes

**Details**:

Read specs/1/13-liquidator.md first. Then read rsx-risk/src/shard.rs,
margin.rs, position.rs, types.rs to understand existing structures.

Create rsx-risk/src/liquidation.rs with:

```rust
// Simple liquidation state machine per user
// LiquidationState tracks: user_id, round, enqueued_at_ns,
// last_order_ns, status (enum: Pending, Active, Done)

pub const MAX_ROUNDS: u8 = 10;
pub const BASE_DELAY_NS: u64 = 1_000_000_000; // 1s
pub const BASE_SLIP_BPS: i64 = 10; // 10 bps

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiquidationStatus {
    Pending,
    Active,
    Done,
}

#[repr(C)]
pub struct LiquidationState {
    pub user_id: u32,
    pub symbol_id: u32,
    pub round: u8,
    pub status: LiquidationStatus,
    pub enqueued_at_ns: u64,
    pub last_order_ns: u64,
}

// LiquidationEngine holds Vec<LiquidationState> (active liquidations)
// and a queue (VecDeque<(u32, u32)>) of (user_id, symbol_id) to process.

pub struct LiquidationEngine {
    pub active: Vec<LiquidationState>,
    pub base_delay_ns: u64,
    pub base_slip_bps: i64,
    pub max_rounds: u8,
}
```

Key functions (keep simple, no generics):

1. `enqueue(&mut self, user_id: u32, symbol_id: u32, now_ns: u64)`
   - Skip if already active for this (user, symbol)
   - Push LiquidationState { round: 0, status: Pending, ... }

2. `maybe_process(&mut self, now_ns: u64, ...) -> Vec<LiquidationOrder>`
   - Walk active list, check if delay elapsed for current round
   - delay = round * base_delay_ns
   - slip_bps = round^2 * base_slip_bps
   - Generate reduce-only limit order with slippage
   - Advance round, mark Done if round > max_rounds

3. `cancel_if_recovered(&mut self, user_id: u32, symbol_id: u32)`
   - Remove from active list if margin recovered

4. `LiquidationOrder` struct: symbol_id, user_id, side, price, qty, is_reduce_only=true

Wire into shard.rs:
- Add `liquidation_engine: LiquidationEngine` to RiskShard
- In run_once(): after margin recalc, check equity < maint_margin
  → call enqueue(). Then call maybe_process() and route orders to ME.
- On fill for liquidating user: check if margin recovered → cancel_if_recovered()

Add to lib.rs: `pub mod liquidation;`

IMPORTANT: Keep it simple. No async. No complex state machines.
Plain Vec, linear scans. This is single-threaded hot path code.

---

### Stage 2: Per-Tick Margin Recalc + Liquidation Trigger

**Goal**: Wire per-tick margin recalc using ExposureIndex.
On price update, recalc margin for exposed users, trigger
liquidation if equity < maintenance margin.
**Files**: rsx-risk/src/shard.rs, rsx-risk/src/margin.rs
**Subagent**: improve
**Dependencies**: [1]
**Verification**:
- [ ] cargo check -p rsx-risk passes
- [ ] cargo test -p rsx-risk -- --skip persist --skip shard_e2e passes

**Details**:

Read rsx-risk/src/margin.rs (ExposureIndex at line ~111),
rsx-risk/src/shard.rs (run_once, process_bbo).

In shard.rs, after processing a BBO update for symbol_id:
1. Get users from exposure_index.users_for_symbol(symbol_id)
2. For each user_id: recalculate margin via calculate()
3. If equity < maint_margin AND no active liquidation:
   enqueue_liquidation(user_id, symbol_id)

Add method to RiskShard:
```rust
fn check_margin_and_liquidate(
    &mut self,
    symbol_id: u32,
    now_ns: u64,
) {
    let users = self.exposure_index
        .users_for_symbol(symbol_id as usize);
    for &user_id in users {
        // collect positions, calc margin
        // if undercollateralized → enqueue
    }
}
```

Call this after every BBO update and after every mark price update.

Also wire maybe_process_liquidations() in run_once() main loop,
after funding check (step 5.5 in RISK.md §10).

Keep it simple: linear scan of exposed users per symbol.
No batching, no deferred processing.

---

### Stage 3: Gateway ORDER_FAILED + Server Heartbeats

**Goal**: Add ORDER_FAILED routing and server-initiated heartbeats
**Files**: rsx-gateway/src/main.rs, rsx-gateway/src/handler.rs,
rsx-gateway/src/ws.rs, rsx-dxs/src/records.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-gateway passes
- [ ] cargo test -p rsx-gateway passes

**Details**:

Read rsx-gateway/src/main.rs, rsx-gateway/src/handler.rs,
rsx-dxs/src/records.rs, specs/1/18-messages.md, specs/1/49-webproto.md.

**Part A: ORDER_FAILED routing**

1. In rsx-dxs/src/records.rs, check if OrderFailedRecord exists.
   If not, add it:
   ```rust
   pub const RECORD_ORDER_FAILED: u16 = 13;

   #[repr(C, align(64))]
   pub struct OrderFailedRecord {
       pub seq: u64,
       pub ts_ns: u64,
       pub user_id: u32,
       pub order_id_hi: u64,
       pub order_id_lo: u64,
       pub reason: u8,
       pub _pad: [u8; 27],
   }
   ```
   Implement CmpRecord for it. reason codes per MESSAGES.md:
   0=INVALID_TICK, 1=INVALID_LOT, 2=INSUFFICIENT_MARGIN,
   3=DUPLICATE_CID, 4=RATE_LIMITED, 5=SYSTEM.

2. In rsx-gateway/src/main.rs, add RECORD_ORDER_FAILED to the
   CMP receive loop. Route to user as WsFrame::OrderUpdate with
   status=3 (failed) and the reason code.

3. In rsx-risk/src/main.rs or shard.rs, when process_order rejects
   (insufficient margin), send OrderFailedRecord via CMP to gateway.

**Part B: Server-initiated heartbeats**

In rsx-gateway/src/main.rs main loop:
1. Track last_heartbeat_ns (Instant)
2. Every 5s (from config.heartbeat_interval_ms), iterate all
   connections and push {H:[timestamp_ns]} frame
3. Track per-connection last_pong_ns
4. If now - last_pong_ns > 10s, close connection

Keep it simple: heartbeat is just a JSON string pushed to each
connection's outbound buffer. No separate timer task.

---

### Stage 4: Liquidation Tests

**Goal**: Comprehensive tests for liquidation engine
**Files**: rsx-risk/tests/liquidation_test.rs (new)
**Subagent**: improve
**Dependencies**: [1, 2]
**Verification**:
- [ ] cargo test -p rsx-risk -- liquidation passes
- [ ] All edge cases covered

**Details**:

Read rsx-risk/src/liquidation.rs (created in Stage 1).
Read specs/1/38-testing-liquidator.md for required test cases.

Create rsx-risk/tests/liquidation_test.rs with tests:

1. enqueue_creates_pending_state
2. enqueue_dedup_same_user_symbol
3. enqueue_allows_different_symbols
4. maybe_process_respects_delay (round 0 = immediate, round 1 = 1s)
5. maybe_process_escalates_slippage (round^2 * base_bps)
6. maybe_process_generates_reduce_only_order
7. maybe_process_marks_done_after_max_rounds
8. cancel_if_recovered_removes_active
9. cancel_if_recovered_noop_when_not_active
10. order_side_matches_position (long → sell, short → buy)
11. order_qty_equals_position_size
12. order_price_includes_slippage (buy: mark*(1+slip), sell: mark*(1-slip))
13. multiple_users_independent_rounds
14. zero_position_skips_order_generation

Keep tests simple: construct LiquidationEngine directly,
call methods, assert results. No mocks needed.

---

### Stage 5: Gateway Tests + Risk Integration Tests

**Goal**: Tests for ORDER_FAILED routing, server heartbeats,
margin recalc trigger
**Files**: rsx-gateway/tests/main_test.rs or handler_test.rs,
rsx-risk/tests/margin_recalc_test.rs (new)
**Subagent**: improve
**Dependencies**: [2, 3]
**Verification**:
- [ ] cargo test -p rsx-gateway passes
- [ ] cargo test -p rsx-risk -- --skip persist --skip shard_e2e passes

**Details**:

**Part A: Gateway tests** (in existing test files or new)

Read rsx-gateway/tests/ to find existing test patterns.

Add to appropriate test file:
1. order_failed_record_layout (size=64, alignment=64)
2. order_failed_record_seq (set/get seq works)
3. serialize_order_failed_to_ws_frame (status=3, reason propagated)

For heartbeat: test is hard without monoio runtime. Document
in tests as TODO comment and in TESTING-GATEWAY.md.

**Part B: Risk margin recalc tests**

Create rsx-risk/tests/margin_recalc_test.rs:
1. bbo_update_triggers_margin_check
2. undercollateralized_user_enqueued_for_liquidation
3. healthy_user_not_enqueued
4. mark_price_update_triggers_margin_check
5. recovered_user_liquidation_cancelled

These test check_margin_and_liquidate() directly on a
constructed RiskShard (or extracted helper function).

---

### Stage 6: Test Coverage Docs + Workspace Verification

**Goal**: Update TESTING-*.md specs with coverage status,
verify full workspace compiles and all tests pass
**Files**: specs/1/38-testing-liquidator.md, specs/1/42-testing-risk.md,
specs/1/37-testing-gateway.md, PROGRESS.md
**Subagent**: improve
**Dependencies**: [4, 5]
**Verification**:
- [ ] cargo check --workspace passes
- [ ] cargo test --workspace --exclude rsx-risk passes
- [ ] cargo test -p rsx-risk -- --skip persist --skip shard_e2e passes
- [ ] PROGRESS.md updated with new percentages

**Details**:

Read each TESTING-*.md spec. For each test case listed:
- If implemented: mark with file:line reference
- If not implemented: mark as TODO with brief reason

Update PROGRESS.md:
- rsx-risk: 54% → ~75% (liquidation basic, margin recalc)
- rsx-gateway: 79% → ~85% (ORDER_FAILED, heartbeats)

Run full workspace verification:
```
cargo check --workspace
cargo test --workspace --exclude rsx-risk
cargo test -p rsx-risk -- --skip persist --skip shard_e2e
```

Fix any compile errors or test failures.
