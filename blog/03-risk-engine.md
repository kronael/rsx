# Real-Time Risk Management in a Perpetual Futures Exchange

The risk engine sits between the gateway and the matching engine.
Every order passes through it. Every fill updates it. If the risk
engine gets margin wrong, the exchange becomes insolvent. This post
covers how RSX handles pre-trade checks, position tracking, margin
calculation, funding, and liquidation -- all in a single-threaded
hot loop on a dedicated core.

## Architecture

Risk is sharded by user. Each shard handles all symbols for its
subset of users. The shard consumes fill events from all matching
engines (one per symbol), filters for its users, and maintains
positions, accounts, and margin state in memory.

```rust
pub struct RiskShard {
    shard_id: u32,
    shard_count: u32,
    max_symbols: usize,
    pub accounts: FxHashMap<u32, Account>,
    pub positions: FxHashMap<(u32, u32), Position>,
    margin: PortfolioMargin,
    pub index_prices: Vec<IndexPrice>,
    pub mark_prices: Vec<i64>,
    exposure: ExposureIndex,
    pub tips: Vec<u64>,
    funding_config: FundingConfig,
    pub last_funding_id: u64,
    taker_fee_bps: Vec<i64>,
    maker_fee_bps: Vec<i64>,
}
```

`FxHashMap` is a hash map optimized for integer keys (no
cryptographic hashing). Positions are keyed by `(user_id,
symbol_id)`. Tips track the last processed sequence number per
symbol for deduplication.

## Pre-Trade Margin Checks

When an order arrives from the gateway, risk must answer one
question: does this user have enough collateral to cover the
worst-case margin requirement if this order fills?

The check:

1. Look up the user's current account and all positions.
2. Calculate the initial margin requirement for the new order.
3. Check: `account.collateral - account.frozen_margin >= required_margin`.
4. If yes, freeze the margin and forward the order to the matching engine.
5. If no, reject the order back to the gateway.

Frozen margin tracks collateral reserved by outstanding orders.
When an order fills or is cancelled, frozen margin is released.

```rust
pub struct Account {
    pub user_id: u32,
    pub collateral: i64,
    pub frozen_margin: i64,
    pub version: u64,
}
```

Everything is `i64` fixed-point. No floats in the margin path.

## Position Tracking

Positions are updated on every fill. The data model:

```rust
pub struct Position {
    pub user_id: u32,
    pub symbol_id: u32,
    pub long_qty: i64,
    pub short_qty: i64,
    pub long_entry_cost: i64,
    pub short_entry_cost: i64,
    pub realized_pnl: i64,
    pub last_fill_seq: u64,
    pub version: u64,
}
```

`long_entry_cost` and `short_entry_cost` are the sum of
`price * qty` for all fills that built the position. This allows
calculating average entry price without storing individual fills.

The core invariant: **position = sum of fills**. After any
recovery, we can verify this by replaying all fills from the
matching engine's WAL and comparing the resulting position
with what is in Postgres. Any mismatch is a critical bug.

## Fill Processing

```rust
pub fn process_fill(&mut self, fill: &FillEvent) {
    let sid = fill.symbol_id as usize;

    // Dedup: skip if seq <= tip for this symbol
    if fill.seq <= self.tips[sid] {
        return;
    }
    // ... process taker and maker sides
}
```

Deduplication is the first check. Each symbol has a monotonically
increasing sequence number. Risk tracks the last processed
sequence per symbol. On replay (after a crash), fills with
`seq <= tip` are skipped. This makes replay idempotent -- the same
fill applied twice has no effect.

For each side (taker and maker), if the user belongs to this shard:

1. Ensure account and position entries exist.
2. Apply the fill to the position (add to long_qty or short_qty,
   update entry cost).
3. Calculate and deduct fees:
   `taker_fee = floor(qty * price * taker_fee_bps / 10_000)`.
   Floor always -- the exchange keeps sub-tick remainders.
4. Release frozen margin for the filled portion.
5. Advance the tip.

Fees use integer division, which truncates toward zero. For
positive values (which fees always are), truncation equals floor.
This is a deliberate choice: the exchange never rounds in the
user's favor.

## Portfolio Margin

RSX uses portfolio margin: all positions across all symbols are
considered together. A user who is long BTC-PERP and short ETH-PERP
has a partially hedged portfolio; the combined margin requirement
is less than the sum of individual requirements.

The margin calculator runs on every fill and periodically when
mark prices update. It computes:

- **Initial margin**: required to open a position.
- **Maintenance margin**: required to keep a position open.
- **Equity**: collateral + unrealized PnL across all positions.

Unrealized PnL uses mark prices from the Mark aggregator, received
via CMP. If the mark price feed is stale, margin checks use the
last known price -- a deliberate choice that trades safety (stale
prices may not reflect reality) for availability (the system keeps
running).

Margin calculations use `i128` intermediate values to prevent
overflow. `qty * price` can exceed `i64::MAX` for large positions
at high prices. The final result is truncated back to `i64`.

## Liquidation

When a user's equity drops below the maintenance margin
requirement, the liquidation engine activates.

```rust
pub struct LiquidationEngine {
    pub active: Vec<LiquidationState>,
    halted_symbols: Vec<bool>,
    base_delay_ns: u64,
    base_slip_bps: i64,
    max_rounds: u32,
}
```

Liquidation happens in rounds. Each round:

1. Check if the user's margin ratio is below the maintenance
   threshold.
2. Generate a liquidation order: a market-like order that closes
   the position, priced with slippage (`base_slip_bps` per round,
   increasing each round).
3. Send the liquidation order to the matching engine.
4. Wait for the fill. If the position is still under-margined,
   start the next round with more slippage.

```rust
pub struct LiquidationOrder {
    pub symbol_id: u32,
    pub user_id: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
}
```

If the liquidation order fails (no liquidity at the slippage
price), the symbol is halted for that user. If `max_rounds` is
reached without closing the position, socialized loss kicks in.

### Insurance Fund

Before socializing losses, the insurance fund absorbs them. The
insurance fund is funded by liquidation profits (when a liquidation
order fills at a better price than the bankruptcy price). If the
insurance fund is depleted, remaining losses are distributed across
all profitable positions in that symbol -- socialized loss.

```rust
pub struct SocializedLoss {
    pub user_id: u32,
    pub symbol_id: u32,
    pub round: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
    pub timestamp_ns: u64,
}
```

The insurance fund balance and all liquidation events are persisted
to Postgres for auditability.

## Funding

Perpetual futures need a funding mechanism to keep the perpetual
price anchored to the spot price. Every 8 hours, a funding
settlement occurs:

- If the perpetual is trading above spot: longs pay shorts.
- If below spot: shorts pay longs.

The key invariant: **funding is zero-sum**. The total amount paid
by longs equals the total received by shorts (and vice versa).
We verify this with:

```sql
SELECT symbol_id, settlement_ts, SUM(amount)
FROM funding_payments
GROUP BY symbol_id, settlement_ts;
```

The sum must be 0. Any deviation is a critical bug.

Funding uses `interval_id = unix_epoch_secs / 28800` as an
idempotency key. Even under clock drift (all hosts run NTP with
<100ms skew), double settlement is impossible because the same
interval_id produces the same payments.

## Persistence

Risk writes to Postgres via a write-behind pattern:

- Positions and accounts are batched and flushed every 10ms.
- Tips are persisted atomically with positions.
- A separate thread handles the Postgres writes; the main loop
  never blocks on I/O.

On crash recovery:

1. Acquire a Postgres advisory lock (exclusive per shard).
2. Load positions, accounts, and tips from Postgres.
3. Request DXS replay from each matching engine starting at
   `tips[symbol_id] + 1`.
4. Process replay fills (same code path as live -- no separate
   recovery logic).
5. On `CaughtUp` for all streams: connect to gateway, go live.

The advisory lock prevents split-brain: at most one risk instance
per shard at any time. Postgres releases the lock automatically
when the holding connection drops (process crash, network failure).
The replica polls `pg_try_advisory_lock()` every 500ms to detect
lock release.

## Replication

Risk supports active-passive replication. The replica receives
fill events from the same matching engines and maintains shadow
state. On master failure, the replica acquires the advisory lock
and promotes itself.

The replica syncs tips with the master via CMP. If the replica's
tips lag behind the master's, it replays from the matching engine
WAL to catch up. The promotion path is the same as the cold-start
recovery path -- load from Postgres, replay from WAL, go live.

## What the Numbers Look Like

Risk processes 234 tests across position tracking, margin
calculation, fee deduction, funding settlement, liquidation
rounds, insurance fund accounting, and replication failover.
The position tracking tests alone cover 60+ edge cases: partial
fills, cross-position netting, fee rounding at boundaries,
reduce-only interactions with liquidation.

The core fill processing path -- dedup check, position update,
fee calculation, margin recalc -- fits in roughly 100 lines of
code. The complexity lives in the edge cases, not the main path.
