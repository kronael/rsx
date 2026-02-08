# Risk Engine Specification

## Context

The risk engine sits between gateway and matching engines. It performs
pre-trade margin checks, ingests fills to update positions, tracks
mark/index prices for risk and funding, and persists state to Postgres.
It must never lose fills and must recover from any crash combination.

## Architecture Overview

```
Binance WS -+  (same process, async task)
             |
           [SPSC: mark prices]
             |
Gateway -[SPSC]-> Risk Shard (main) -[SPSC]-> Matching Engines
                       |        ^                     |
                       |        | [SPSC: BBO + fills]  |
                       |        +---------------------+
                       |
                  [SPSC: tip sync]
                       |
                  Risk Shard (replica)
                       ^
                       | [SPSC: BBO + fills from MEs directly]
```

- **N risk shards** by `user_id` hash, each with a dedicated replica
- Each shard consumes **all symbols** (filters fills for its users)
- Hot path is single-threaded, busy-spin on dedicated core
- Postgres write-behind on separate thread (10ms flush)

## Components

### 1. Fill Ingestion

- SPSC consumer ring from each matching engine
- Each fill contains `taker_user_id`, `maker_user_id`, `symbol_id`,
  `seq_no`
- Filter: `user_in_shard(user_id)` via range or bitmask check
- Dedup: skip if `seq_no <= tips[symbol_id]`
- Apply fill to both taker and maker positions (if in shard)
- Advance per-symbol tip after processing
- Push to persistence rings (positions, fills, tips)
- Push to replica ring (tip sync)

### 2. Position Manager

In-memory `FxHashMap<(user_id, symbol_id), Position>`:

```rust
struct Position {
    user_id: u32,
    symbol_id: u32,
    long_qty: i64,          // fixed-point lot units
    short_qty: i64,
    long_entry_cost: i64,   // sum(price * qty) for avg price
    short_entry_cost: i64,
    realized_pnl: i64,
    last_fill_seq: u64,
    version: u64,           // monotonic, for CAS on Postgres upsert
}
```

In-memory `FxHashMap<u32, Account>`:

```rust
struct Account {
    user_id: u32,
    collateral: i64,        // fixed-point
    frozen_margin: i64,     // reserved by open orders
    version: u64,
}
```

### 3. Margin Calculator (Portfolio Margin)

Portfolio margin: considers all positions across all symbols for a
user.

**Recalculate on every tick.** Sharding keeps load manageable.

**Exposure index** -- Vec indexed by symbol (u8/u16):
```rust
// Symbols have a compact index (u8 or u16, limits max symbols)
// Vec of user_ids with open positions per symbol
exposure: Vec<Vec<u32>>,  // exposure[symbol_idx] = [user_ids...]
```

Updated on fill (add user) and position close (remove user).
On each price tick (BBO or mark price update), recalculate
margin for all users with exposure in that symbol. Scale
horizontally by adding more user shards.

```rust
struct PortfolioMargin {
    // Per-symbol risk parameters (loaded from config)
    symbol_params: Vec<SymbolRiskParams>,
}

struct SymbolRiskParams {
    initial_margin_rate: i64,      // fixed-point bps
    maintenance_margin_rate: i64,
    max_leverage: i64,
}

impl PortfolioMargin {
    /// Full portfolio margin for a user across all positions
    fn calculate(&self, positions: &[(u32, &Position)],
        mark_prices: &Vec<i64>) -> MarginState;

    /// Pre-trade: can this order be placed given current portfolio?
    fn check_order(&self, account: &Account,
        positions: &[(u32, &Position)],
        order: &OrderRequest,
        mark_prices: &Vec<i64>)
        -> Result<i64, RejectReason>;

    /// Is this user below maintenance margin?
    fn needs_liquidation(&self, state: &MarginState) -> bool;
}
```

Margin recalculated on every price tick for all exposed users.

### 4. Price Feeds

**From matching engines** (SPSC ring, same as fills):
- BBO updates: `(symbol_id, best_bid, best_bid_qty, best_ask,
  best_ask_qty)`
- Risk engine calculates **index price** per symbol:
  `index = (best_bid * ask_qty + best_ask * bid_qty)
           / (bid_qty + ask_qty)`
- O(1) per BBO update, stored in `Vec<IndexPrice>`

**From Binance** (WebSocket, async task in same process):
- Async tokio task connects to Binance mark price WS
- Handles reconnects, parsing
- Pushes `(symbol_id, mark_price)` to risk hot path via SPSC ring
- Risk engine reads mark prices into `Vec<i64>`
- Used for margin/risk calculations (unrealized PnL, liquidation)

### 5. Funding Engine

Logically part of risk engine, could be extracted later.

- **Funding rate** = f(mark_price, index_price)
  - `premium = (mark_price - index_price) / index_price`
  - Rate clamped to bounds, formula TBD per symbol config
- **Interval**: 8h (periodic), configurable per symbol
- **Application**: at interval boundary, iterate all positions for
  each symbol, apply funding payment/charge:
  - Long pays short when funding rate > 0 (mark > index)
  - Short pays long when funding rate < 0 (mark < index)
  - `funding_payment = position_qty * mark_price * funding_rate`
- Rate calculated continuously (updated on each price tick)
- Applied atomically at settlement time
- Funding payments persisted to Postgres (append-only)

### 6. Pre-Trade Risk Check

```
process_order(order):
    // Collect all positions for this user
    user_positions = get_user_positions(order.user_id)
    // Full portfolio margin recalc with latest mark prices
    margin_needed = portfolio_margin.check_order(
        account, user_positions, order, mark_prices)
    if err: reject to gateway
    account.frozen_margin += margin_needed
    route order to matching engine for symbol_id
```

On fill: update position, margin recalculated on next price tick.
On ORDER_DONE: release frozen margin for that order.

### 7. Per-Tick Margin Recalc

On each price update (BBO or mark price) for a symbol:

```
fn on_price_update(symbol_idx: u16):
    for user_id in &exposure[symbol_idx]:
        positions = get_user_positions(user_id)
        state = portfolio_margin.calculate(
            positions, &mark_prices)
        accounts[user_id].margin_state = state
        if portfolio_margin.needs_liquidation(&state):
            enqueue_liquidation(user_id)
```

- Runs on every tick -- sharding keeps it fast
- Only checks users with exposure in the updated symbol
- Scale horizontally by adding more user shards

## Persistence

### Postgres Schema

```sql
CREATE TABLE positions (
    user_id      INT NOT NULL,
    symbol_id    INT NOT NULL,
    long_qty     BIGINT NOT NULL DEFAULT 0,
    short_qty    BIGINT NOT NULL DEFAULT 0,
    long_entry_cost   BIGINT NOT NULL DEFAULT 0,
    short_entry_cost  BIGINT NOT NULL DEFAULT 0,
    realized_pnl      BIGINT NOT NULL DEFAULT 0,
    version      BIGINT NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, symbol_id)
);

CREATE TABLE accounts (
    user_id        INT PRIMARY KEY,
    collateral     BIGINT NOT NULL DEFAULT 0,
    frozen_margin  BIGINT NOT NULL DEFAULT 0,
    version        BIGINT NOT NULL DEFAULT 0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE fills (
    fill_id       BIGINT NOT NULL,
    symbol_id     INT NOT NULL,
    taker_user_id INT NOT NULL,
    maker_user_id INT NOT NULL,
    taker_order_id BYTEA NOT NULL,
    maker_order_id BYTEA NOT NULL,
    side          SMALLINT NOT NULL,
    price         BIGINT NOT NULL,
    qty           BIGINT NOT NULL,
    seq_no        BIGINT NOT NULL,
    timestamp_ns  BIGINT NOT NULL,
    inserted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
) PARTITION BY RANGE (timestamp_ns);

CREATE INDEX idx_fills_symbol_seq ON fills (symbol_id, seq_no);

CREATE TABLE tips (
    instance_id  INT NOT NULL,
    symbol_id    INT NOT NULL,
    last_seq_no  BIGINT NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (instance_id, symbol_id)
);

CREATE TABLE funding_payments (
    user_id      INT NOT NULL,
    symbol_id    INT NOT NULL,
    amount       BIGINT NOT NULL,
    rate         BIGINT NOT NULL,
    settlement_ts TIMESTAMPTZ NOT NULL,
    inserted_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### Write Patterns

All persistence happens on a **separate write-behind thread**:
- Hot path pushes to rtrb SPSC rings (non-blocking)
- Write-behind thread drains rings every 10ms (or on threshold)
- Single Postgres transaction per flush:
  1. Positions: batched `UPSERT` (no version guard -- advisory lock
     guarantees single writer per shard)
  2. Fills: `COPY` binary (fastest bulk insert)
  3. Tips: batched `UPSERT` (no guard -- single writer)
- `synchronous_commit = on` for durability

### Backpressure

Per WAL.md:
- Persistence ring full -> stall hot path (ME stalls upstream)
- Flush lag > 10ms -> stall hot path
- Replica ring full -> stall hot path (configurable, 100ms bound)

## Replication & Failover

### Main Behavior

1. Acquire Postgres advisory lock: `pg_advisory_lock(shard_id)`
2. Load positions, accounts, tips from Postgres
3. Request replay from each ME: `seq_no > tips[symbol_id]`
4. Process replay fills (same code path as live)
5. On `ReplayDone` for all symbols: connect gateway, go live
6. Main loop: poll ME rings -> poll gateway -> renew lease (~1s)
7. Send every processed tip advance to replica via SPSC

### Replica Behavior

1. Try advisory lock (expected to fail, main holds it)
2. Load same positions/tips from Postgres as baseline
3. Connect SPSC consumers to all matching engines (direct)
4. Connect SPSC consumer to main (tip sync channel)
5. Replica loop:
   - Buffer fills from MEs into `Vec<Fill>` per symbol
     (already ordered)
   - On tip from main: apply buffered fills up to that tip
   - Poll `pg_try_advisory_lock(shard_id)` every ~500ms
   - If acquired: main is dead -> promote

### Promotion (Replica -> Main)

1. Acquire advisory lock (main's connection dropped, lock released)
2. Apply all remaining buffered fills beyond main's last tip
3. Connect outbound to gateway
4. Start write-behind worker
5. Resume processing new orders

### Recovery: Both Crash

1. New instance acquires advisory lock
2. Reads positions + tips from Postgres (up to 10ms stale)
3. Requests replay from MEs: `seq_no > tips[symbol_id]`
4. MEs serve from 10min in-memory buffer, or WAL for older
5. Replays to current, goes live, starts new replica

### Matching Engine Failover

- ME replica starts sending (lease-based authority)
- ME main shuts up when it detects replica's authoritative stream
- Risk engine deduplicates by `(symbol_id, seq_no)` -- no restart
  needed

## Main Loop Pseudocode

```
loop {
    // 1. Fills from all MEs (highest priority, NEVER skip)
    for ring in me_rings:
        while let Ok(event) = ring.try_pop():
            match event:
                Fill(f) => process_fill(f)
                BBO(b)  => stash_bbo(b)  // save latest, process later

    // 2. New orders from gateway (NEVER skip)
    while let Ok(order) = gateway_ring.try_pop():
        process_order(order)

    // 3. Mark prices from Binance feeder
    while let Ok(mp) = binance_ring.try_pop():
        mark_prices[mp.symbol_id] = mp.price

    // 4. BBO price updates (LAST, skippable under load)
    //    Only process latest BBO per symbol (skip stale)
    //    Triggers margin recalc for all exposed users
    if has_budget():
        for (sym, bbo) in drain_stashed_bbos():
            update_index_price(bbo)
            recalc_margins_for_symbol(sym)

    // 5. Funding check (amortized, every 8h)
    maybe_settle_funding()

    // 6. Lease renewal (every ~1s)
    maybe_renew_lease()
}
```

## File Organization

```
crates/rsx-risk/src/
    main.rs           -- entrypoint, config, process setup
    shard.rs          -- RiskShard struct, main loop
    position.rs       -- Position, apply_fill
    account.rs        -- Account, collateral
    margin.rs         -- PortfolioMargin, exposure_index, liquidation
    price.rs          -- IndexPrice (size-weighted mid), mark price
    funding.rs        -- FundingEngine, rate calc, settlement
    fill.rs           -- FillEvent types
    tip.rs            -- Tip tracking
    lease.rs          -- Advisory lock acquire/renew/release
    replica.rs        -- Replica loop, fill buffer, promotion
    persist.rs        -- Write-behind worker, Postgres batching
    replay.rs         -- Replay request/response, cold start
    binance.rs        -- Binance WS mark price feeder (async task)
    config.rs         -- TOML config
    types.rs          -- Price, Qty, type aliases
    risk_utils.rs     -- helpers
```

## Performance Targets

| Path | Target | Operations |
|------|--------|------------|
| Fill processing | <1us | HashMap lookup, arithmetic, ring push |
| Pre-trade check | <5us | portfolio margin across all positions |
| Per-tick margin | <10us/user | recalc all exposed users on tick |
| BBO -> index price | <100ns | arithmetic only |
| Postgres flush | every 10ms | batched UPSERT + COPY |
| Failover detection | ~500ms | advisory lock poll interval |
| Replay catch-up | <5s | depends on gap size |

## Implementation Phases

Each phase is standalone, demonstrable, and benchmarkable.

### Phase 1: Position + Margin Math (no I/O)

Pure functions for position tracking, portfolio margin, index price,
and funding rate. No rings, no Postgres, no network.

**Demo:** CLI binary that takes a sequence of fills from stdin (JSON),
prints position state and margin after each fill.

**Files:** `position.rs`, `account.rs`, `margin.rs`, `price.rs`,
`funding.rs`, `types.rs`

#### Unit Tests

```rust
// position.rs -- core
apply_buy_fill_opens_long
apply_sell_fill_opens_short
apply_opposing_fill_reduces_position
apply_fill_closing_position_realizes_pnl
avg_entry_price_weighted_correctly
multiple_fills_same_side_accumulate
fill_larger_than_position_flips_side
zero_qty_after_exact_close

// position.rs -- edge cases
flip_long_to_short_single_fill
flip_short_to_long_single_fill
fill_at_same_price_no_pnl
realized_pnl_accumulates_across_fills
self_trade_taker_and_maker_same_user
max_qty_no_overflow
max_price_no_overflow
position_version_increments_per_fill

// margin.rs -- core
portfolio_margin_single_position
portfolio_margin_multi_symbol
portfolio_margin_long_short_offset
check_order_sufficient_margin_accepts
check_order_insufficient_margin_rejects
needs_liquidation_below_maintenance
needs_liquidation_above_maintenance_ok
frozen_margin_reserved_on_order
frozen_margin_released_on_done

// margin.rs -- edge cases
check_order_exactly_at_margin_limit_accepts
check_order_one_unit_over_limit_rejects
margin_with_zero_collateral_rejects_all
margin_with_no_positions_all_available
margin_unrealized_pnl_affects_equity
margin_mark_price_zero_handled
margin_max_leverage_enforced
frozen_margin_across_multiple_pending_orders
order_done_partial_fill_releases_remaining_frozen
order_failed_releases_all_frozen

// price.rs -- core
index_price_size_weighted_mid
index_price_balanced_book_equals_mid
index_price_imbalanced_favors_thicker_side

// price.rs -- edge cases
index_price_one_side_zero_qty
index_price_both_sides_zero_qty
index_price_max_values_no_overflow
index_price_spread_zero_equals_price

// funding.rs -- core
funding_rate_mark_above_index_positive
funding_rate_mark_below_index_negative
funding_rate_clamped_to_bounds
funding_payment_long_pays_when_positive
funding_payment_short_pays_when_negative
funding_zero_position_no_payment

// funding.rs -- edge cases
funding_rate_mark_equals_index_zero
funding_zero_sum_across_all_users
funding_with_position_opened_mid_interval
funding_extreme_divergence_clamped
funding_settlement_idempotent
funding_index_price_zero_handled

// exposure index -- core
exposure_add_user_on_fill
exposure_remove_user_on_close
exposure_no_duplicate_entries

// exposure index -- edge cases
exposure_user_in_multiple_symbols
exposure_close_one_symbol_keeps_others
exposure_symbol_idx_out_of_bounds_panics
exposure_empty_vec_for_unused_symbol
```

#### Benchmarks

```rust
bench_apply_fill_to_position        // target <100ns
bench_portfolio_margin_10_positions  // target <10us
bench_portfolio_margin_50_positions
bench_index_price_calculation        // target <50ns
bench_exposure_lookup_100_users      // target <50ns
bench_exposure_lookup_1000_users
```

### Phase 2: Fill Ingestion + Main Loop (mocked rings)

RiskShard with main loop processing fills, orders, and BBO from
mocked SPSC rings. No Postgres. State in-memory only.

**Demo:** Binary that creates a shard with mocked ME producers.
Producers generate random fills. Shard processes, prints stats
(fills/sec, margin recalcs/sec, position count).

**Files:** `shard.rs`, `fill.rs`, `tip.rs`, `config.rs`

#### Unit Tests

```rust
// fill ingestion -- core
fill_for_shard_user_updates_position
fill_for_other_shard_ignored
fill_both_users_in_shard_updates_both
fill_dedup_by_seq_no
fill_advances_tip_per_symbol
tip_monotonic_never_decreases

// fill ingestion -- edge cases
fill_seq_no_gap_still_advances_tip
fill_seq_no_zero_first_ever
fill_for_unknown_symbol_advances_tip_only
fill_taker_in_shard_maker_not
fill_maker_in_shard_taker_not
fill_self_trade_same_user_both_sides
fill_rapid_sequence_same_symbol
fill_interleaved_symbols
tip_not_advanced_on_duplicate_fill

// main loop ordering
fills_processed_before_bbo
orders_processed_after_fills
bbo_skipped_under_load
stale_bbo_replaced_by_latest
mark_price_update_triggers_margin_recalc
empty_rings_no_crash
burst_fills_then_idle

// pre-trade risk -- core
order_accepted_margin_sufficient
order_rejected_margin_insufficient
frozen_margin_accumulates_on_multiple_orders
order_done_releases_frozen_margin

// pre-trade risk -- edge cases
order_for_user_not_in_shard_rejected
order_while_user_being_liquidated_rejected
order_reducing_position_always_accepted
order_with_zero_qty_rejected
order_duplicate_id_within_dedup_window
order_cancel_releases_frozen_margin
```

#### E2E Tests

```rust
shard_processes_1000_fills_positions_correct
shard_multi_symbol_tips_advance_independently
shard_margin_recalc_on_bbo_update
shard_order_accept_reject_flow
shard_liquidation_detected_on_price_drop
shard_bbo_skip_under_fill_pressure
shard_multiple_users_same_symbol
shard_user_opens_closes_reopens
shard_position_flip_through_fills
shard_fill_updates_exposure_index
shard_order_accepted_then_rejected_margin_used
shard_cancel_restores_margin_for_next_order
shard_mark_price_divergence_triggers_liquidation
shard_funding_settlement_at_interval
shard_idle_no_resource_leak
```

#### Benchmarks

```rust
bench_shard_fill_throughput_1_symbol     // target >1M fills/sec
bench_shard_fill_throughput_10_symbols
bench_shard_fill_throughput_100_symbols
bench_pretrade_check_latency             // target <5us
bench_margin_recalc_100_users_1_symbol   // target <10us/user
bench_margin_recalc_100_users_10_symbols
bench_bbo_processing                     // target <200ns
bench_main_loop_idle                     // target <1us
```

### Phase 3: Persistence (testcontainers Postgres)

Write-behind worker persisting positions, fills, and tips to
Postgres. Recovery: cold start from Postgres.

**Demo:** Binary that processes fills, flushes to Postgres, crashes
(kill -9), restarts, loads state from Postgres, verifies positions
match pre-crash state (within 10ms bounded loss).

**Files:** `persist.rs`, `replay.rs`, migrations

#### Unit Tests

```rust
worker_drains_ring_on_interval
worker_batches_multiple_position_updates
worker_deduplicates_same_position_in_batch
worker_single_transaction_per_flush
```

#### Integration Tests

```rust
persist_positions_roundtrip
persist_fills_copy_batch
persist_tips_roundtrip
persist_funding_payments_append
cold_start_loads_positions
cold_start_loads_tips
recovery_bounded_loss_10ms
upsert_idempotent_on_replay
fill_partitioning_works
persist_handles_pg_connection_drop
persist_backpressure_ring_full
persist_empty_batch_no_transaction
persist_position_overwritten_by_later_version
cold_start_with_empty_postgres
```

#### Benchmarks

```rust
bench_flush_100_positions       // target <5ms
bench_flush_1000_positions      // target <15ms
bench_copy_1000_fills           // target <5ms
bench_copy_10000_fills          // target <20ms
bench_load_10k_positions        // target <500ms
bench_load_100k_positions       // target <2s
bench_sustained_flush_10ms_interval_60s
```

### Phase 4: Replication + Failover

Main/replica pair with fill buffering, tip sync, promotion, and
advisory lock lease management.

**Demo:** Start main + replica. Feed fills. Kill main (kill -9).
Observe replica promotes, continues processing. Show no fill loss.
Start new replica for the promoted main.

**Files:** `replica.rs`, `lease.rs`

#### Unit Tests

```rust
buffer_fills_in_order
drain_up_to_seq_no
drain_partial_leaves_remainder
buffer_empty_drain_returns_empty
buffer_multi_symbol_independent
lease_acquire_succeeds_when_free
lease_acquire_fails_when_held
lease_released_on_connection_drop
```

#### Integration Tests

```rust
main_acquires_lease_replica_cannot
main_crash_replica_promotes
replica_applies_buffered_fills_on_promotion
replica_state_matches_main
both_crash_recovery_from_postgres
me_failover_dedup_by_seq_no
promotion_no_fill_loss
split_brain_prevented_by_advisory_lock
```

#### Benchmarks

```rust
bench_failover_detection_time        // target <600ms
bench_replica_drain_1000_fills       // target <100us
bench_replica_drain_10000_fills      // target <1ms
bench_promotion_total_time           // target <1s
```

### Phase 5: Full System (all components)

Complete risk engine with all components integrated. Multi-symbol,
multi-user, with funding, Binance feed, persistence, replication.

**Demo:** Full system with N mocked matching engines, M users.
Dashboard showing: fills/sec, margin recalcs/sec, Postgres flush
latency, position count, funding rates. Inject price crash ->
observe liquidations.

**Files:** `main.rs`, `binance.rs`, all integrated

#### E2E Tests

```rust
full_lifecycle_order_fill_position_margin
multi_user_multi_symbol_positions_independent
funding_settlement_8h_correct
funding_rate_updates_on_price_change
binance_feed_updates_mark_price
binance_reconnect_on_disconnect
liquidation_cascade_under_price_crash
bbo_skip_under_heavy_fill_load
order_rejected_during_liquidation
shard_boundary_fill_taker_shard0_maker_shard1
all_symbols_simultaneous_bbo_update
mark_price_stale_binance_disconnect
rapid_open_close_cycles
max_users_per_shard_performance
fill_burst_after_idle_period
funding_with_position_changes_during_interval
```

#### Integration Tests

```rust
full_crash_recovery_end_to_end
backpressure_slow_postgres
multi_shard_same_fill_different_users
funding_persisted_to_postgres
concurrent_shard_leases_independent
```

#### Smoke Tests

```rust
risk_engine_responds_to_order
risk_engine_positions_update_on_fill
risk_engine_margin_query_returns
risk_engine_funding_rate_available
risk_engine_replica_running
```

#### Benchmarks

```rust
bench_e2e_fill_to_margin_latency          // target <15us
bench_sustained_1m_fills_10_symbols_100_users
    // target >100K fills/sec/shard
bench_margin_recalc_1000_users_10_symbols  // target <10ms
bench_memory_10k_positions                 // target <10MB
bench_memory_100k_positions                // target <100MB
bench_funding_settlement_10k_positions     // target <50ms
bench_cold_start_10k_positions_50_symbols  // target <5s
```

## Correctness Invariants

Verified across all test levels:

1. **Fills never lost** -- sum of applied fills = sum of ME-emitted
   fills (for shard users). After any crash/recovery, no fill missing.

2. **Position = sum of fills** -- `position.long_qty = sum(fill.qty
   where side=buy)`. Verified after every test scenario.

3. **Tips monotonic** -- `tips[symbol_id]` never decreases. After
   recovery, tip = last persisted seq_no.

4. **Margin consistent with positions** -- margin recalc from scratch
   matches incremental state. Verified periodically in long-running
   tests.

5. **Funding zero-sum** -- `sum(funding_payments) = 0` across all
   users per symbol per interval. Longs pay exactly what shorts
   receive (and vice versa).

6. **Exposure index consistent** -- `exposure[sym]` contains exactly
   users with `position.qty != 0`. No phantom entries, no missing
   entries.

7. **Advisory lock exclusive** -- at most one main per shard at any
   time. Verified via Postgres `pg_locks` query in tests.

8. **Seq_no dedup prevents double-counting** -- replay of
   already-processed fills = no position change.

## Test Data Patterns

### Normal Market
- 10 symbols, 100 users per shard
- 1-2 tick spread, balanced buy/sell
- 1K fills/sec per symbol

### Price Crash (50% drop)
- Mark price drops 50% over 10s
- Triggers liquidations for leveraged users
- Tests margin recalc under rapid price changes

### Funding Divergence
- Mark price 5% above index for extended period
- Tests funding rate calculation and settlement

### Shard Boundary
- Fill where taker hash -> shard 0, maker hash -> shard 1
- Both shards process independently

### Replay Burst
- 100K fills replayed on cold start
- Tests replay throughput and correctness
