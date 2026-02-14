# Position Tracking Edge Cases

Comprehensive catalog of edge cases for position tracking across
the RSX exchange. Cross-references RISK.md §2, GUARANTEES.md §8.1,
CONSISTENCY.md §4, LIQUIDATOR.md §4-6.

**Key invariant:** `position = sum(fills)` always holds after
replay (GUARANTEES.md §8.1).

## Table of Contents

- [1. Position State Transitions](#1-position-state-transitions)
- [2. Arithmetic Edge Cases](#2-arithmetic-edge-cases)
- [3. Multi-User Interactions](#3-multi-user-interactions)
- [4. Crash and Recovery Edge Cases](#4-crash-and-recovery-edge-cases)
- [5. Liquidation Edge Cases](#5-liquidation-edge-cases)
- [6. Price Feed Edge Cases](#6-price-feed-edge-cases)
- [7. Fee and Collateral Edge Cases](#7-fee-and-collateral-edge-cases)
- [8. Concurrency and Ordering Edge Cases](#8-concurrency-and-ordering-edge-cases)
- [9. Symbol Config Edge Cases](#9-symbol-config-edge-cases)
- [10. Replay and Reconciliation Edge Cases](#10-replay-and-reconciliation-edge-cases)
- [11. Network and Partition Edge Cases](#11-network-and-partition-edge-cases)
- [12. Summary of Critical Invariants](#12-summary-of-critical-invariants)
- [13. References](#13-references)

---

## 1. Position State Transitions

### 1.1 Empty Position (Zero Quantity)

**Definition:** `long_qty = 0 AND short_qty = 0`

**Behavior:**
- `net_qty()` returns 0
- `notional()` returns 0 (regardless of mark price)
- `avg_entry()` returns 0 (no division by zero)
- `unrealized_pnl()` returns 0 (short-circuits early)
- `is_empty()` returns true

**Edge cases:**
- Empty position after exact close (qty matches exactly)
- Empty position on fresh account (never traded)
- Empty position with realized PnL non-zero (closed with profit/loss)
- Multiple fills sum to zero (buy 10, sell 5, sell 5)

**Margin implications:**
- No initial margin required (notional = 0)
- No maintenance margin required
- Available margin = equity - frozen_margin
- Cannot trigger liquidation (no position to close)

**Exposure tracking:**
- User removed from `exposure[symbol_idx]` on close
- Re-added on next fill that opens position
- Empty vector `exposure[symbol_idx] = []` for unused symbols

**Test coverage:** TESTING-RISK.md lines 76, 144

---

### 1.2 Position Flip (Long → Short or Short → Long)

**Definition:** Fill qty exceeds opposing position, crossing zero.

**Example:**
```
Initial: long_qty=10, short_qty=0, long_entry_cost=1000
Fill: SELL 15 @ 110
Result: long_qty=0, short_qty=5, short_entry_cost=550
```

**Two-step process (position.rs lines 33-89):**
1. **Close opposing side** (qty=10):
   - Proportion of entry cost: `(1000 * 10 / 10) = 1000`
   - Realized PnL: `(110 * 10 - 1000) = +100`
   - Zero out long side: `long_qty=0, long_entry_cost=0`
2. **Open new side** (remaining=5):
   - New short: `short_qty=5, short_entry_cost=(110*5)=550`
   - Entry price for new side = fill price (110)

**Edge cases:**
- Flip at same price as avg entry (zero realized PnL on close)
- Flip with large qty (100x opposing position)
- Flip with minimal remaining (close 99.99%, open 0.01%)
- Multiple consecutive flips in same direction
- Self-trade flip (taker and maker same user)

**Invariants:**
- Old position fully closed (qty=0, entry_cost=0)
- New position opened at fill price
- Realized PnL += close_pnl (incremental)
- `version` increments exactly once per fill

**Test coverage:** TESTING-RISK.md lines 67-70

---

### 1.3 Partial Close (Reduce Position Size)

**Definition:** Fill qty < opposing position qty, reduces but
doesn't cross zero.

**Example:**
```
Initial: long_qty=10, short_qty=0, long_entry_cost=1000
Fill: SELL 3 @ 110
Result: long_qty=7, short_qty=0, long_entry_cost=700
```

**Proportional cost reduction:**
- Close cost: `(1000 * 3 / 10) = 300`
- Realized PnL: `(110 * 3 - 300) = +30`
- Remaining cost: `1000 - 300 = 700`
- Avg entry unchanged: `700 / 7 = 100`

**Edge cases:**
- Repeated partial closes (asymptotic to zero)
- Partial close after flip (reduces new side)
- Partial close with rounding (fixed-point division)
- Partial close at different prices (accumulating realized PnL)

**Test coverage:** TESTING-RISK.md line 60

---

### 1.4 Accumulation (Same Side Repeated Fills)

**Definition:** Multiple fills on same side, no crossing.

**Example:**
```
Initial: long_qty=10, short_qty=0, long_entry_cost=1000
Fill 1: BUY 5 @ 105
  -> long_qty=15, long_entry_cost=1525
Fill 2: BUY 2 @ 110
  -> long_qty=17, long_entry_cost=1745
```

**Weighted average entry:**
- Entry cost accumulates: `sum(price * qty)`
- Avg entry: `entry_cost / qty`
- After fills above: `1745 / 17 = 102.647...` (truncated)

**Edge cases:**
- Many small accumulations (precision drift from truncation)
- Accumulation at widely varying prices (avg entry smoothing)
- Accumulation with zero opposing side (most common case)
- Accumulation after partial close (rebuilding position)

**Fixed-point precision:**
- Uses i128 intermediate for cost calculation (position.rs:59)
- Truncates to i64 on final result
- Rounding error bounded per fill, NOT cumulative
  (replay produces same result)

**Test coverage:** TESTING-RISK.md line 62

---

## 2. Arithmetic Edge Cases

### 2.1 Overflow Prevention

**All position arithmetic uses i128 intermediate** to prevent
overflow:

**Critical operations:**
- `entry_cost = price * qty` (position.rs:59, 88)
- `notional = |net_qty| * mark_price` (position.rs:101)
- `unrealized_pnl = net_qty * (mark - avg_entry)` (position.rs:125)
- Funding payment: `qty * mark_price * rate` (funding.rs:38)

**Max safe values (with i128):**
- Max price: `i64::MAX / 2` (~4.6e18 in tick units)
- Max qty: `i64::MAX / 2`
- Product fits in i128 without overflow

**Overflow checks at API boundary (pre-trade risk check):**
```rust
// RISK.md §6: order notional check
order_notional = checked_mul(order.price, order.qty)?;
  -> rejects if overflow
```

**No overflow checks on hot path** (position update, margin
recalc). Assumes pre-trade validation prevents impossible
values. If overflow occurs on hot path = critical bug.

**Test coverage:** TESTING-RISK.md line 73

---

### 2.2 Division by Zero Prevention

**All divisions check denominator:**

**Avg entry price (position.rs:105-113):**
```rust
pub fn avg_entry(&self) -> i64 {
    let nq = self.net_qty();
    if nq > 0 {
        self.long_entry_cost / self.long_qty  // safe: nq>0 implies long_qty>0
    } else if nq < 0 {
        self.short_entry_cost / self.short_qty  // safe: nq<0 implies short_qty>0
    } else {
        0  // empty position
    }
}
```

**Index price (RISK.md §4):**
```rust
// Risk engine price.rs
if bid_qty + ask_qty == 0:
    return last_known_index  // no division
index = (bid * ask_qty + ask * bid_qty) / (bid_qty + ask_qty)
```

**Funding rate (when index=0):**
- Premium undefined if `index_price = 0`
- Funding engine skips settlement if no valid index
- Test: `funding_index_price_zero_handled` (TESTING-RISK.md:129)

**Test coverage:** TESTING-RISK.md line 76

---

### 2.3 Negative Collateral

**Acceptable under leverage:**
- User can have `collateral < 0` if unrealized losses exceed
  initial capital
- Margin check: `equity = collateral + sum(unrealized_pnl)`
- Liquidation trigger: `equity < maintenance_margin`

**Example:**
```
collateral = 1000
position: long 10 BTC @ 100 (notional = 1000)
mark price drops to 50
unrealized_pnl = 10 * (50 - 100) = -500
equity = 1000 + (-500) = 500
```

If mark drops further to 10:
```
unrealized_pnl = 10 * (10 - 100) = -900
equity = 1000 + (-900) = 100
```

If mark drops to 0 (extreme):
```
unrealized_pnl = -1000
equity = 0
```

**Liquidation prevents deep negative:**
- Triggered at `equity < maintenance_margin`
- Maintenance margin = notional * maint_rate (e.g., 2.5%)
- For 1000 notional @ 2.5%: liquidation at equity < 25

**Insurance fund covers socialized loss:**
- If liquidation cannot close position (no counterparty)
- After max rounds at max slippage (LIQUIDATOR.md §9)
- Remaining loss = `|equity|` if equity < 0 after closure

**Test coverage:** GUARANTEES.md §8.4

---

### 2.4 Maximum Values

**Max qty without overflow (with i128 intermediate):**
- Single position: `i64::MAX / 2` (~9.2e18 lot units)
- Practical limit: symbol config `max_position_qty`

**Max price without overflow:**
- `i64::MAX / 2` tick units
- Practical limit: symbol config `max_price`

**Max entry cost:**
- `max_qty * max_price` fits in i128
- Truncates to i64 on storage

**Max notional for margin:**
- Portfolio margin sums across all symbols
- `sum(|net_qty_i| * mark_price_i)` uses i128
- Liquidation prevents unbounded growth

**Test coverage:** TESTING-RISK.md lines 73, 112

---

## 3. Multi-User Interactions

### 3.1 Self-Trade (Taker and Maker Same User)

**Definition:** User's order matches their own resting order
on the book.

**Matching engine behavior:**
- ME does NOT prevent self-trade (no user_id check on match)
- Emits single FILL with `taker_user_id = maker_user_id`

**Risk engine processing:**
- Receives fill via CMP/UDP from ME
- Applies fill TWICE to same user's position:
  - Once as taker (one side)
  - Once as maker (opposite side)
- Net position change = 0 (sides cancel)

**Fee impact:**
- Taker fee charged (e.g., 5 bps)
- Maker rebate credited (e.g., -2 bps)
- Net fee = taker_fee + maker_fee (e.g., 3 bps)
- User pays net fee for self-trade

**Position state after self-trade:**
- If both sides same qty: position unchanged (wash)
- Realized PnL changes (if fill price ≠ entry price)
- Entry cost may shift due to proportional close/reopen
- Collateral reduced by net fee

**Example:**
```
Initial position: long 10 @ 100 (cost=1000)
Self-trade: sell 10 @ 110 (taker), buy 10 @ 110 (maker)

Step 1 (taker sell):
  close long: rpnl = (110*10 - 1000) = +100
  position: 0
Step 2 (maker buy):
  open long: entry_cost = 110*10 = 1100
  position: long 10 @ 110
Net: position qty unchanged, entry price now 110, rpnl +100
Fees: collateral -= (taker_fee + maker_rebate)
```

**Liquidation self-trade:**
- Liquidation order can match user's own resting order
- Same wash behavior
- Reduces position on both sides simultaneously
- May recover margin if fill price favorable

**Test coverage:** TESTING-RISK.md line 72

---

### 3.2 Simultaneous Fills (Same User, Different Symbols)

**Scenario:** User has positions in BTC-PERP and ETH-PERP.
Fills arrive in same ME event batch (or near-simultaneously
via CMP/UDP from different MEs).

**Risk engine processing (single-threaded per shard):**
- Processes fills sequentially (FIFO order)
- Each fill updates position in-memory immediately
- Margin recalc AFTER both fills applied (per-tick, not
  per-fill)

**Portfolio margin impact:**
- First fill changes BTC position -> margin state stale
- Second fill changes ETH position -> margin state stale
- Next BBO/mark tick triggers full portfolio recalc across
  both symbols

**Liquidation timing:**
- If first fill pushes user below maintenance margin,
  liquidation NOT enqueued until margin recalc (next tick)
- If second fill recovers margin, no liquidation triggered

**No race condition:**
- Risk main loop is single-threaded (RISK.md §7)
- Fills and liquidation processing are serialized
- No concurrent reads of partial state

**Test coverage:** implied by single-threaded design

---

### 3.3 Concurrent Orders (Same User, Same Symbol)

**Scenario:** User submits two orders for same symbol before
first fills.

**Gateway + Risk flow:**
1. Order A arrives at gateway -> routed to risk
2. Order B arrives at gateway -> routed to risk
3. Risk checks margin for A (holds position snapshot)
4. Risk freezes margin for A
5. Risk checks margin for B (uses updated frozen_margin from A)
6. Risk freezes additional margin for B
7. Both orders routed to ME

**Frozen margin accumulation:**
- `frozen_margin = sum(margin_needed_i + fee_reserve_i)`
  across all pending orders
- Available margin = `equity - initial_margin - frozen_margin`
- Second order may be rejected if insufficient available
  margin after first order's reservation

**Fill processing:**
- Fill A arrives -> updates position, margin recalculated
- ORDER_DONE A -> releases frozen_margin for A
- Fill B arrives -> updates position (new notional)
- ORDER_DONE B -> releases frozen_margin for B

**Edge case: second order increases frozen margin beyond
available:**
```
equity = 1000, initial_margin = 200, available = 800
Order A: needs 400 margin -> frozen = 400, available = 400
Order B: needs 500 margin -> REJECTED (available = 400)
```

**Test coverage:** TESTING-RISK.md line 98

---

## 4. Crash and Recovery Edge Cases

### 4.1 Position Staleness (Risk Crash)

**Scenario:** Risk engine crashes. Postgres has positions from
10ms ago. Matching engine has processed fills in last 10ms.

**Recovery flow (RISK.md §9, GUARANTEES.md §2.2):**
1. New risk instance starts
2. Loads positions + tips from Postgres (stale by up to 10ms)
3. Requests DXS replay from ME: `from_seq = tips[symbol_id] + 1`
4. ME serves fills from last 10min WAL retention
5. Risk replays fills, updates positions in-memory
6. When caught up: persists new tips, goes live

**Edge case: tip in Postgres is stale:**
- Risk replays more fills than necessary
- Fill dedup: `if seq <= tips[symbol_id]: skip`
- Idempotent: replaying duplicate fill = no position change
- Convergence: position = sum(fills) always after replay

**Data loss bound:**
- Fills: 0ms (ME WAL has complete history)
- Positions: 10ms (reconstructed from fills)

**Test coverage:** GUARANTEES.md §8.1

---

### 4.2 Dual Crash (Risk Master + Replica)

**Scenario:** Both risk instances crash within 10ms. Postgres
may not have committed last batch.

**Worst-case position staleness:** 100ms
- Risk flush interval: 10ms
- Postgres commit lag: up to 100ms (if batch in flight)

**Recovery:**
1. New risk instance starts
2. Postgres has positions from up to 100ms ago
3. Risk replays fills from `tips[symbol_id] + 1`
4. ME WAL retains fills for 10min (plenty of buffer)
5. Position = sum(fills) converges

**No position drift:**
- Even if tip is 100ms stale, replaying fills is
  deterministic
- Same fill sequence -> same final position
- Fixed-point arithmetic is deterministic (no floating-point
  error accumulation)

**Test coverage:** GUARANTEES.md §3.2

---

### 4.3 Replay with Position Flip

**Scenario:** Position in Postgres shows long 10. Replay
includes fills that flip to short 5.

**Replay sequence:**
```
Postgres: long_qty=10, short_qty=0, tips[BTC]=1000
Replay fills:
  seq=1001: SELL 15 @ 110 (flips to short 5)
  seq=1002: BUY 2 @ 105 (reduces short to 3)
```

**Position reconstruction:**
- Start from Postgres state (long 10)
- Apply fill 1001: flip logic executes (close long, open short)
- Apply fill 1002: reduce short
- Final: short_qty=3

**Idempotency check:**
- If fill 1001 already in Postgres (seq <= tip), skip
- If both fills already applied, skip both
- Dedup by seq prevents double-application

**Realized PnL accumulation:**
- Postgres has realized_pnl from fills up to tip
- Replay fills add incremental realized PnL
- Final realized_pnl = Postgres value + replay incremental

**Test coverage:** implied by `position = sum(fills)` invariant

---

### 4.4 Replay with Funding Settlement

**Scenario:** Risk crashes. During downtime, funding interval
boundary passes (e.g., 00:00 UTC). Replay needed.

**Funding settlement is NOT in fill stream:**
- Funding payments are separate (not FILL records)
- Funding engine runs in risk process (RISK.md §5)
- On cold start: Risk checks last settled interval

**Missed interval detection:**
```rust
// On startup
last_settled = load_last_funding_settlement(db)
current_interval = unix_epoch_secs() / 28800
if current_interval > last_settled:
    for interval in (last_settled+1)..=current_interval:
        settle_funding(interval)
```

**Idempotency via interval_id:**
- Each settlement keyed by `(symbol_id, interval_id)`
- Duplicate settlement for same interval = no-op
- Test: `funding_settlement_idempotent` (TESTING-RISK.md:128)

**Collateral reconciliation:**
- Funding payments affect collateral, not position qty
- After replay: `collateral = Postgres + fills fees + funding`
- Position qty unchanged by funding

**Test coverage:** TESTING-RISK.md lines 128, 130

---

## 5. Liquidation Edge Cases

### 5.1 Margin Recovery During Liquidation

**Scenario:** User in liquidation. Fill arrives on liquidation
order, improving unrealized PnL enough to recover margin.

**Flow (LIQUIDATOR.md §5):**
1. User below maintenance margin -> enqueued for liquidation
2. Round 1 liquidation orders placed (1bp slippage)
3. Liquidation order fills (e.g., closes 50% of position at
   favorable price)
4. Risk applies fill -> updates position
5. Margin recalc: `equity >= maintenance_margin`
6. Liquidation status -> Cancelled
7. Remaining liquidation orders cancelled
8. User can submit new orders (no longer in liquidation)

**Edge case: recovery on partial fill:**
- Liquidation order for qty=10
- Partial fill qty=3 at favorable price
- Margin recovered with position still open
- Liquidation cancelled, user retains 70% of position

**Edge case: recovery on price tick (no fill):**
- User in liquidation with long position
- Mark price rises (unrealized PnL improves)
- Per-tick margin recalc detects recovery
- Liquidation cancelled before any fills

**Test coverage:** TESTING-LIQUIDATOR.md (implied by lifecycle)

---

### 5.2 Liquidation Order Matches User's Resting Order

**Scenario:** User has limit buy @ 100 resting. Goes into
liquidation. Liquidation order is sell @ 99 (1bp slippage).
Liquidation order matches user's own resting order.

**Result:**
- Self-trade (§3.1 above)
- Position qty unchanged (both sides cancel)
- Realized PnL may change (if entry price ≠ fill price)
- Fees charged (net of taker + maker)

**Liquidation impact:**
- Position not closed by this fill (wash trade)
- Margin may recover if fill price favorable (realized gain)
- If margin NOT recovered: escalate to round 2

**Cancellation of non-liquidation orders:**
- On entering liquidation: cancel all user's pending
  non-liquidation orders (LIQUIDATOR.md §4)
- Releases frozen margin (may recover margin immediately)
- Prevents this edge case in most scenarios

**Test coverage:** self-trade test + liquidation lifecycle

---

### 5.3 Reduce-Only Clamping by ME

**Definition:** Liquidation orders have `reduce_only = true`.
ME clamps qty to current position size (ORDERBOOK.md §6.5).

**Scenario:** Risk engine generates liquidation order for
qty=10. User's position is 8 (partially closed by prior fill
or self-trade).

**ME behavior:**
- Clamps order qty to 8 (position size)
- Only 8 can fill (cannot exceed position)
- ORDER_DONE with `filled_qty = 8`

**Risk engine response:**
- Receives fill for qty=8 (not 10)
- Updates position: closed fully
- Removes from pending_orders
- Liquidation status -> Completed (all positions closed)

**Edge case: position reduced to zero before liquidation
order placed:**
- Round 1 order placed for qty=10
- Before fill: user manually closes position (or self-trade)
- ME clamps liquidation order to 0 -> ORDER_DONE immediately
  with qty=0
- Risk engine detects: position empty, liquidation Completed

**Test coverage:** TESTING-LIQUIDATOR.md line 40

---

### 5.4 Frozen Margin During Liquidation

**Liquidation orders do NOT freeze margin:**
- User already underwater (equity < maintenance margin)
- No point reserving margin (insufficient collateral)

**Non-liquidation orders rejected during liquidation:**
- User in Active liquidation status
- New order submission -> rejected immediately
- Reject reason: `order_while_user_being_liquidated_rejected`
- Test: TESTING-LIQUIDATOR.md (implied)

**Frozen margin release on entering liquidation:**
- All pending non-liquidation orders cancelled
- Frozen margin for those orders released
- May recover margin enough to avoid liquidation
- Re-check margin after release (LIQUIDATOR.md §4)

**Edge case: large frozen margin masks underwater state:**
```
collateral = 1000
unrealized_pnl = -500
frozen_margin = 600 (large pending order)
equity = 1000 + (-500) = 500
available = 500 - 200 (initial_margin) - 600 (frozen) = -300

Trigger: equity < maintenance margin (500 < 600)? No, user OK.
But: available < 0 (cannot place more orders)

On pending order cancel (timeout, user cancel, or liquidation entry):
frozen_margin = 0
available = 500 - 200 = 300
User recovers (was never truly underwater, just over-leveraged
on pending orders)
```

**Test coverage:** TESTING-LIQUIDATOR.md line 143

---

## 6. Price Feed Edge Cases

### 6.1 Mark Price Unavailable

**Fallback chain (RISK.md §4, LIQUIDATOR.md §3):**
1. **Mark price** from aggregator (DXS consumer)
2. **Index price** from BBO (risk engine calculates)
3. **Last known mark price** (cached)

**Scenario: mark aggregator offline:**
- No `MarkPriceEvent` received
- Risk uses index price from BBO (local fallback)
- Margin calculations continue (no stall)

**Scenario: BBO also unavailable (symbol halted):**
- No BBO updates from ME
- Risk uses last known mark price (may be stale)
- Liquidation continues with stale price (better than no
  liquidation)

**Index price = 0 edge case:**
- If `bid_qty + ask_qty = 0` (no liquidity)
- Index price = last known index
- If no BBO ever received: index = 0
- Margin calculation: uses mark price (primary)

**Test coverage:** TESTING-RISK.md lines 95, 96, 111

---

### 6.2 Mark Price = 0

**Scenario:** Mark price aggregator has no valid sources,
publishes mark_price = 0.

**Impact:**
- Notional = `|net_qty| * 0 = 0`
- Unrealized PnL = `net_qty * (0 - avg_entry)` = large
  negative (if long) or large positive (if short)
- Equity = collateral + unrealized_pnl (may trigger
  liquidation)

**Handling:**
- Mark price = 0 is treated as valid (not filtered out)
- Liquidation triggered if equity < maintenance margin
- Liquidation order price = `0 +/- slippage` (may be
  negative for sell, clamped to 0 by ME)

**Practical prevention:**
- Mark aggregator should NOT publish 0 unless all sources
  offline
- If all sources offline: no MarkPriceEvent published (fallback
  to index)

**Test coverage:** TESTING-RISK.md line 96

---

### 6.3 Crossed Mark vs Index (Mark > Index for Short)

**Funding rate impact:**
- Premium = `(mark - index) / index`
- If mark > index: positive premium
- Longs pay shorts (shorts receive funding)

**Margin calculation:**
- Uses mark price for unrealized PnL (not index)
- Short position with mark > index -> unrealized loss
  (bought low, mark now high)
- May trigger liquidation even if profitable at index price

**Liquidation price:**
- Liquidation order uses mark price +/- slippage
- Short liquidation: buy at `mark + slippage`
- May fill at higher price than index (immediate loss)

**No arbitrage prevention:**
- Risk engine does not prevent crossed prices
- External market forces should converge mark and index
- Funding mechanism incentivizes arbitrage

**Test coverage:** funding tests (TESTING-RISK.md §funding)

---

## 7. Fee and Collateral Edge Cases

### 7.1 Negative Fee (Maker Rebate)

**Definition:** Maker fee can be negative (rebate).

**Example:**
- Maker fee rate = -2 bps
- Fill: qty=100, price=1000
- Notional = 100,000
- Maker fee = `floor(100,000 * -2 / 10,000) = -20`
- Collateral credited: `collateral -= (-20)` = `collateral += 20`

**Fee formula (RISK.md §1):**
```rust
// Floor always — exchange keeps the sub-tick remainder
taker_fee = floor(qty * price * taker_fee_bps / 10_000)
maker_fee = floor(qty * price * maker_fee_bps / 10_000)
  // Negative if rebate
```

**Overflow prevention:**
- Uses i128 intermediate: `qty * price * fee_bps`
- Truncates to i64
- Bounded by pre-trade notional check

**Test coverage:** TESTING-RISK.md line 101 (implied)

---

### 7.2 Fee Reserve in Pre-Trade Check

**Pre-trade margin calculation (RISK.md §6):**
```rust
order_notional = order.price * order.qty
order_im = order_notional * initial_margin_rate
fee_reserve = order_notional * taker_fee_bps / 10_000
  // Worst-case: assume taker fee (higher than maker)
margin_needed = order_im + fee_reserve
if available < margin_needed: reject
```

**Fee reserve frozen:**
- Worst-case taker fee reserved (even if order may be maker)
- On fill: actual fee deducted from collateral, frozen margin
  released
- If maker fill: rebate credited, excess frozen margin released

**Edge case: fee reserve exceeds available margin:**
```
available = 100
order_notional = 10,000
order_im = 200 (2% initial margin rate)
fee_reserve = 50 (0.5% taker fee)
margin_needed = 250 -> REJECTED (available = 100)
```

User needs 250 available to place order with notional 10,000,
even though only 200 will be used for margin (fee is 50).

**Test coverage:** TESTING-RISK.md line 101

---

### 7.3 Collateral Exhaustion

**Scenario:** User has positive unrealized PnL but negative
collateral (fees exceeded initial capital).

**Example:**
```
initial collateral = 1000
fees paid (taker) = 1200 (heavy trading)
collateral = 1000 - 1200 = -200

position: long 10 @ 100 (entry_cost = 1000)
mark price = 150
unrealized_pnl = 10 * (150 - 100) = +500
equity = -200 + 500 = 300
```

**Margin check:**
- Equity = 300 > 0 (user solvent)
- If maintenance margin = 25 (2.5% of 1000): equity > maint,
  OK
- User can continue trading (equity positive)

**Withdrawal check (future feature):**
- Available for withdrawal = `equity - initial_margin -
  frozen_margin`
- If equity < initial_margin: cannot withdraw (must close
  positions first)

**Extreme case: equity < 0:**
- Liquidation triggered
- If liquidation fails (no counterparty): insurance fund
  covers loss

**Test coverage:** implied by margin tests

---

## 8. Concurrency and Ordering Edge Cases

### 8.1 Fill Before ORDER_DONE

**Invariant (CONSISTENCY.md §2):** Fills precede ORDER_DONE
for the same order.

**Scenario:** Large order fills in multiple chunks.

**Event sequence (guaranteed by ME):**
```
FILL { order_id=A, qty=5, seq=100 }
FILL { order_id=A, qty=3, seq=101 }
ORDER_DONE { order_id=A, total_filled=8, seq=102 }
```

**Risk engine processing:**
- seq=100: apply fill, update position, version++
- seq=101: apply fill, update position, version++
- seq=102: release frozen margin for order A

**Edge case: ORDER_DONE arrives out-of-order (UDP):**
- CMP/UDP may deliver out-of-order
- Risk dedup by seq: processes in seq order even if arrival
  out-of-order
- CMP NACK + resend ensures no gaps

**Test coverage:** CONSISTENCY.md verification

---

### 8.2 BBO Delayed (Stale Index Price)

**Scenario:** Risk engine receives fills, but BBO updates lag
(ME -> Risk CMP/UDP congestion).

**Impact:**
- Index price stale (last BBO from 100ms ago)
- Margin calculated with stale index (if mark unavailable)
- Liquidation decision may be incorrect (using old price)

**Mitigation:**
- Mark price is primary (not index)
- BBO staleness affects index only
- Per-tick margin recalc catches up when BBO arrives
- Liquidation based on mark (DXS consumer, separate channel)

**Worst case:**
- Both mark AND BBO stale
- Risk uses last known mark price (may be very stale)
- Liquidation delayed but NOT prevented
- Better to liquidate late than never

**Test coverage:** index price tests (TESTING-RISK.md §price)

---

### 8.3 Tip Persistence Lag

**Scenario:** Risk processes fills, but tip not yet persisted
to Postgres (write-behind lag).

**In-memory tip:** `tips[symbol_id] = 1050` (latest processed)

**Postgres tip:** `tips[symbol_id] = 1000` (from 10ms ago)

**On crash before Postgres flush:**
1. New risk instance loads tip = 1000
2. Requests replay from seq=1001
3. ME serves fills 1001-1050 (from WAL)
4. Risk replays, deduplicates any already-applied fills
5. Converges to same position

**Idempotency:**
- Fills 1001-1050 may be already reflected in Postgres
  positions (if positions flushed but tips not yet)
- Dedup by seq: if `seq <= in_memory_tip: skip`
- In-memory tip updated on each fill, Postgres tip lags

**Tip as optimization:**
- Reduces replay window (smaller DXS request)
- NOT source of truth (position = sum(fills) is truth)

**Test coverage:** GUARANTEES.md §8.2

---

## 9. Symbol Config Edge Cases

### 9.1 Config Update During Liquidation

**Scenario:** User in liquidation. Symbol config updated (e.g.,
maintenance margin rate changes).

**Config propagation (RISK.md §1):**
- ME emits `CONFIG_APPLIED` event on config update
- Risk consumes CONFIG_APPLIED via CMP/UDP
- Risk updates in-memory symbol config
- Risk forwards CONFIG_APPLIED to Gateway (cache sync)

**Margin recalculation:**
- Next per-tick margin recalc uses new rates
- If new maintenance margin rate lower: user may recover
- If new rate higher: more users may enter liquidation

**Liquidation in progress:**
- Liquidation orders already placed use old config
- Next round uses new config (slippage formula unchanged,
  but mark price may differ)
- No retroactive config changes (orders in flight not
  cancelled)

**Edge case: config disables symbol during liquidation:**
- Symbol halted (max_price = 0 or similar)
- ME rejects liquidation orders (symbol halted)
- Risk detects ORDER_FAILED -> pauses liquidation for that
  symbol (LIQUIDATOR.md §4)
- When symbol re-enabled: liquidation resumes

**Test coverage:** config update tests (TESTING-MATCHING.md)

---

### 9.2 Max Position Exceeded (Historical Data)

**Scenario:** Symbol config reduces `max_position_qty`. User's
existing position exceeds new limit.

**Pre-trade check:**
- New orders rejected if `position + order.qty > max_position_qty`
- Reduce-only orders allowed (even if position > max)

**Existing position:**
- NOT force-closed by config change
- User can only reduce position (via reduce-only orders or
  liquidation)
- Grandfathered in until user voluntarily reduces

**Liquidation with oversized position:**
- Liquidation orders are reduce-only (always allowed)
- Liquidation proceeds normally

**Test coverage:** pre-trade check tests

---

## 10. Replay and Reconciliation Edge Cases

### 10.1 Gap in Fill Sequence

**Scenario:** Risk replays fills from DXS. Detects gap in seq.

**Example:**
```
Expected seq: 1001
Received: 1001, 1002, 1004 (1003 missing)
```

**CMP protocol handling (CMP.md):**
- Consumer detects gap via seq comparison
- Sends NACK to producer (ME)
- ME resends missing fill (from WAL buffer)
- Consumer receives 1003, continues

**Risk behavior:**
- If gap detected: pause processing for that symbol
- Request resend via CMP NACK
- Wait for missing fill(s)
- Resume when gap filled

**Edge case: fill lost in WAL (critical bug):**
- If ME WAL is corrupt or lost: cannot replay
- Violates GUARANTEES.md (0ms fill loss guarantee)
- Recovery: restore from snapshot + DXS Recorder archive

**Test coverage:** CMP gap detection (TESTING-CMP.md)

---

### 10.2 Position Reconciliation Mismatch

**Definition:** Periodic check that `position = sum(fills)`.

**Reconciliation query (GUARANTEES.md §8.1):**
```sql
SELECT user_id, symbol_id,
  SUM(CASE WHEN side = 0 THEN qty ELSE 0 END) AS fills_long,
  SUM(CASE WHEN side = 1 THEN qty ELSE 0 END) AS fills_short
FROM fills
WHERE seq <= (SELECT last_seq FROM tips
              WHERE symbol_id = fills.symbol_id
              AND instance_id = ?)
GROUP BY user_id, symbol_id;
```

Compare with `positions` table. Any mismatch = critical bug.

**Possible causes (all bugs):**
- Fill applied twice (dedup failed)
- Fill skipped (seq gap not detected)
- Position update logic wrong (flip/close/accumulate)
- Arithmetic overflow (i128 -> i64 truncation wrong)

**Remediation:**
- Halt trading for affected user/symbol
- Rebuild position from fills (sum from genesis)
- Compare with expected, identify divergence point
- Fix bug, redeploy, verify

**Test coverage:** Run after every recovery test

---

### 10.3 Funding Reconciliation (Zero-Sum Violation)

**Invariant (GUARANTEES.md §8.5):** For each funding interval,
`sum(funding_payments) = 0`.

**Reconciliation query:**
```sql
SELECT symbol_id, settlement_ts, SUM(amount)
FROM funding_payments
WHERE settlement_ts = ?
GROUP BY symbol_id, settlement_ts;
```

Sum must be 0 (within rounding error).

**Possible causes (all bugs):**
- Fixed-point rounding error accumulated (should be bounded)
- User skipped in funding loop (iteration bug)
- Funding rate formula wrong (premium calculation)
- Position qty wrong at settlement time

**Remediation:**
- Identify which users received incorrect funding
- Manual adjustment (credit/debit collateral)
- Fix bug, redeploy

**Test coverage:** TESTING-RISK.md line 125

---

## 11. Network and Partition Edge Cases

### 11.1 Risk Isolated from ME (Fills Buffered)

**Scenario:** Network partition between Risk and ME. ME
continues processing fills, Risk offline.

**ME behavior:**
- Continues matching orders
- Writes fills to WAL (local disk, no network needed)
- DXS server retains fills in WAL (10min retention)

**Risk recovery when partition heals:**
- Risk reconnects to ME DXS server
- Requests replay from `tips[symbol_id] + 1`
- ME serves fills from WAL (up to 10min backlog)
- Risk catches up, goes live

**Partition duration > 10min:**
- ME WAL rotates, old fills offloaded to Recorder
- Risk must replay from snapshot + full WAL archive
- Recovery time: minutes (depends on archive size)

**Test coverage:** GUARANTEES.md §5.2

---

### 11.2 Risk Isolated from Postgres (Backpressure)

**Scenario:** Network partition between Risk and Postgres.
Risk continues processing fills, but cannot persist.

**Risk behavior:**
- Write-behind buffer fills up
- When buffer lag > 100ms: Risk stalls fill processing
  (GUARANTEES.md §6.1)
- Backpressure propagates: ME -> Risk -> Gateway

**User-visible impact:**
- Gateway rejects new orders (Risk not accepting)
- Fills continue for existing orders (ME still matching)
- When buffer threshold hit: ME stalls (ring full)

**Recovery when partition heals:**
- Risk resumes flushing to Postgres
- Backlog drained in batches
- Backpressure released
- Trading resumes

**Test coverage:** backpressure tests

---

## 12. Summary of Critical Invariants

**Position = sum(fills)** (GUARANTEES.md §8.1)
- Verified after every recovery scenario
- Reconciliation query compares Postgres positions vs fills
- Any mismatch = critical bug

**Tips monotonic** (GUARANTEES.md §8.2)
- `tips[symbol_id]` never decreases
- After recovery: in-memory tip >= Postgres tip

**Margin consistent** (GUARANTEES.md §8.3)
- Recalc from scratch = incremental margin state
- Verified periodically (every 1000 fills in tests)

**Funding zero-sum** (GUARANTEES.md §8.5)
- `sum(funding_payments) = 0` per interval
- Verified after every settlement

**Fills idempotent** (GUARANTEES.md §8.6)
- Replaying same fill twice = no position change
- Dedup by seq prevents double-application

**Frozen margin released** (position tracking impact)
- ORDER_DONE releases frozen margin
- Liquidation entry cancels all orders (releases frozen)

**Reduce-only enforcement** (ME prevents position growth)
- Liquidation orders clamped to position size
- Self-trade on reduce-only = wash (no qty change)

---

## 13. References

- RISK.md: Position tracking formulas, margin calculation
- GUARANTEES.md: System-wide invariants and recovery bounds
- CONSISTENCY.md: Event ordering and fan-out guarantees
- LIQUIDATOR.md: Liquidation lifecycle and edge cases
- ORDERBOOK.md: Reduce-only enforcement, position tracking in ME
- TESTING-RISK.md: Comprehensive unit/e2e/integration tests
- CMP.md: Gap detection and NACK/resend protocol
- DXS.md: WAL replay and retention guarantees
