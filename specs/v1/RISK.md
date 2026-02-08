# Risk Engine Specification

## Context

The risk engine sits between gateway and matching engines. It performs
pre-trade margin checks, ingests orderbook events to update positions,
tracks mark/index prices for risk and funding, and persists state to a
durable store (Postgres in v1). Order intents at ingress are not WAL’d;
they may be lost on risk crash before execution. Risk must recover from
orderbook WAL replay.

## Architecture Overview

```
Mark Aggregator -+  (DXS consumer, see MARK.md)
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

### 1. Orderbook Event Ingestion

- SPSC consumer ring from each matching engine
- Each fill contains `taker_user_id`, `maker_user_id`, `symbol_id`,
  `seq`
- Filter: `user_in_shard(user_id)` via range or bitmask check
- Dedup: skip if `seq <= tips[symbol_id]`
- Apply fill to both taker and maker positions (if in shard)
- Fee calculation on each fill:
  - `taker_fee = floor(qty * price * taker_fee_bps / 10_000)`
  - `maker_fee = floor(qty * price * maker_fee_bps / 10_000)`
    (Floor always — exchange keeps the sub-tick remainder.
    Integer division in Rust truncates toward zero, which
    equals floor for positive values.)
  - Deduct: `taker.collateral -= taker_fee`
  - Deduct: `maker.collateral -= maker_fee` (negative fee =
    rebate credited)
  - Fee rates from symbol config (METADATA.md)
  - Persist fee with fill record
- Apply order_done/cancel/failed to release frozen margin
- Advance per-symbol tip after processing
- Push to persistence rings (positions, fills, tips)
- Push to replica ring (tip sync)
- Apply config updates from matcher `CONFIG_APPLIED` events.
  On cold start, Risk bootstraps current config from Postgres
  (ME writes applied config to `symbol_config_applied` table
  on each CONFIG_APPLIED event). CONFIG_APPLIED on the DXS
  stream is an optimization for live sync; Postgres is source
  of truth for cold start. See METADATA.md.
- Forward `CONFIG_APPLIED` to Gateway for cache sync

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

**Formulas** (all values fixed-point integers):

Per-position:
```
net_qty  = long_qty - short_qty  (signed, + = long)
notional = |net_qty| * mark_price
avg_entry = entry_cost / |net_qty|
  (long_entry_cost if net_qty > 0, short_entry_cost if < 0)
unrealized_pnl = net_qty * (mark_price - avg_entry)
```

Per-user (across all positions):
```
equity     = collateral + sum(unrealized_pnl_i)
initial_margin = sum(notional_i * initial_margin_rate_i)
maint_margin   = sum(notional_i * maintenance_margin_rate_i)
available  = equity - initial_margin - frozen_margin
```

Pre-trade (section 6):
```
order_im  = order_notional * initial_margin_rate
order_fee = order_notional * taker_fee_bps / 10_000
accept if: available >= order_im + order_fee
```

Liquidation trigger (section 7):
```
if equity < maint_margin: enqueue_liquidation(user_id)
```

Edge cases:
- Empty position (qty=0): upnl=0, notional=0
- Position flip: close old at fill price (realize PnL),
  open new with entry = fill price. Two-step in apply_fill.
- Mark price unavailable: use index price (section 4)

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

struct MarginState {
    equity: i64,
    unrealized_pnl: i64,
    initial_margin: i64,
    maintenance_margin: i64,
    available_margin: i64,
}
```

Margin recalculated on every price tick for all exposed users.

### 4. Price Feeds

**From matching engines** (SPSC ring, same as fills):
- BBO updates: `(symbol_id, best_bid, best_bid_qty, best_ask,
  best_ask_qty)`
- BBO is also derived by MARKETDATA from its shadow orderbook
  (see [MARKETDATA.md](MARKETDATA.md) section 4)
- Risk engine calculates **index price** per symbol:
  `index = (best_bid * ask_qty + best_ask * bid_qty)
           / (bid_qty + ask_qty)`
- If `bid_qty + ask_qty == 0`: use last known index
- If only one side has qty: use that side's price
- If no BBO ever received: use mark price
- O(1) per BBO update, stored in `Vec<IndexPrice>`

**From Mark Price Aggregator** (DXS consumer, see [MARK.md](MARK.md)):
- Risk engine connects as a DXS consumer to the mark price
  aggregator ([DXS.md](DXS.md) section 6)
- Receives `MarkPriceEvent` records via DXS streaming
- DXS consumer callback pushes to risk hot path via SPSC ring
  (same integration point as before)
- Risk engine reads mark prices into `Vec<i64>`
- Used for margin/risk calculations (unrealized PnL, liquidation)
- Fallback: if mark price aggregator has zero sources (all stale),
  no `MarkPriceEvent` is published. Risk uses index price from BBO.

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
- Settlement: UTC 00:00, 08:00, 16:00
- **Idempotency key:** `interval_id = unix_epoch_secs / 28800`
  (28800 = 8 hours). Each funding settlement is keyed by
  `(symbol_id, interval_id)`. Duplicate settlement for same
  interval_id is a no-op.
- **Clock requirement:** NTP required on all hosts. Maximum
  allowed clock skew: 100ms. Funding settlement uses wall
  clock; skew >100ms could cause interval_id mismatch between
  components.
- Missed intervals: settle on next startup
- Mark price at settlement = latest available
- Funding payments persisted to Postgres (append-only)

### 6. Pre-Trade Risk Check

```
process_order(order):
    // Reduce-only: pass through to ME (ME enforces)
    // is_liquidation: skip margin check entirely
    if order.is_liquidation:
        route order to matching engine
        return  // no frozen margin, no margin check

    // Collect all positions for this user
    user_positions = get_user_positions(order.user_id)
    // Full portfolio margin recalc with latest mark prices
    margin_needed = portfolio_margin.check_order(
        account, user_positions, order, mark_prices)
    if err: reject to gateway
    // Reserve worst-case taker fee
    order_notional = order.price * order.qty
    fee_reserve = order_notional * taker_fee_bps / 10_000
    margin_needed += fee_reserve
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
            // see LIQUIDATOR.md
```

**No liquidation race condition:** The risk engine main loop is
single-threaded. Fills and liquidation round processing are
serialized: a fill updates the position before
`maybe_process_liquidations()` runs. No concurrent reads of
partial state. The single-threaded design eliminates the race
between fill arrival and escalation decision.

- Runs on every tick -- sharding keeps it fast
- Only checks users with exposure in the updated symbol
- Scale horizontally by adding more user shards

## Persistence

**Retention (v1):** Postgres keeps per-user order state and positions. History
retention in Postgres is a v1 choice; v2 will move long-term history off
Postgres.

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
    taker_fee     BIGINT NOT NULL DEFAULT 0,
    maker_fee     BIGINT NOT NULL DEFAULT 0,
    seq        BIGINT NOT NULL,
    timestamp_ns  BIGINT NOT NULL,
    inserted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
) PARTITION BY RANGE (timestamp_ns);

CREATE INDEX idx_fills_symbol_seq ON fills (symbol_id, seq);

CREATE TABLE tips (
    instance_id  INT NOT NULL,
    symbol_id    INT NOT NULL,
    last_seq  BIGINT NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (instance_id, symbol_id)
);

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
- Persistence ring full -> **must stall** hot path (ME stalls upstream)
- Flush lag > 10ms -> **must stall** hot path
- Replica ring full -> **must stall** hot path (configurable, 100ms bound)

## Replication & Failover

**Failover guarantees:** See [GUARANTEES.md](../../GUARANTEES.md) for complete
specification of failover behavior, data loss bounds (single crash: 10ms,
dual crash: 100ms), and recovery time objectives.

**Failover procedures:** See [RECOVERY-RUNBOOK.md](../../RECOVERY-RUNBOOK.md)
§2.2 for step-by-step recovery from Risk master crash, and §4 for dual
crash scenarios.

### Main Behavior

1. Acquire Postgres advisory lock: `pg_advisory_lock(shard_id)`
2. Load positions, accounts, tips from Postgres
3. Request replay from each ME via DXS consumer
   ([DXS.md](DXS.md) section 6): `from_seq = tips[symbol_id] + 1`
4. Process replay fills (same code path as live)
5. On `CaughtUp` for all streams: connect gateway, go live
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
2. Apply all buffered fills **up to the last tip** (promotion invariant)
3. Connect outbound to gateway
4. Start write-behind worker
5. Resume processing new orders

### Recovery: Both Crash

**Data loss bound:** 100ms positions (worst case if both crash before Postgres
flush). Fills are NEVER lost — ME WAL retains all fills for 10min, Risk replays
from `tips[symbol_id] + 1`.

1. New instance acquires advisory lock
2. Reads positions + tips from Postgres (up to 10ms stale)
3. Requests replay via DXS consumer ([DXS.md](DXS.md)):
   `from_seq = tips[symbol_id] + 1`
4. MEs serve from 10min WAL retention (DXS.md section 2)
5. Replays to current, goes live, starts new replica

**Idempotent replay:** Fill processing is idempotent — replaying
a fill with `seq <= tips[symbol_id]` is a no-op (dedup by seq).
Tip persistence is an optimization (reduces replay window). Even
if tip is stale, `position = sum(fills)` is always rebuildable
from ME WAL. The system converges to correct state regardless of
tip staleness.

**100ms loss bound proof:** Risk flushes to Postgres every 10ms. If both
instances crash before flush, max loss = 10ms of position updates. If Postgres
ALSO slow to commit (transaction in progress), max loss extends to 100ms. After
recovery, all positions are reconstructed from ME fills (position = sum(fills)).

### Matching Engine Failover

- ME replica starts sending (lease-based authority)
- ME main shuts up when it detects replica's authoritative stream
- Risk engine deduplicates by `(symbol_id, seq)` -- no restart
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

    // 3. Mark prices from aggregator (DXS consumer, see MARK.md)
    while let Ok(mp) = mark_ring.try_pop():
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

    // 5.5. Liquidation processing (see LIQUIDATOR.md)
    maybe_process_liquidations()

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
    liquidation.rs    -- LiquidationEngine, state, order gen (LIQUIDATOR.md)
    fill.rs           -- FillEvent types
    tip.rs            -- Tip tracking
    lease.rs          -- Advisory lock acquire/renew/release
    replica.rs        -- Replica loop, fill buffer, promotion
    persist.rs        -- Write-behind worker, Postgres batching
    replay.rs         -- Replay request/response, cold start
    mark_consumer.rs  -- DXS consumer for mark prices (MARK.md)
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

### Phase 2: Fill Ingestion + Main Loop (mocked rings)

RiskShard with main loop processing fills, orders, and BBO from
mocked SPSC rings. No Postgres. State in-memory only.

**Demo:** Binary that creates a shard with mocked ME producers.
Producers generate random fills. Shard processes, prints stats
(fills/sec, margin recalcs/sec, position count).

**Files:** `shard.rs`, `fill.rs`, `tip.rs`, `config.rs`

### Phase 3: Persistence (testcontainers Postgres)

Write-behind worker persisting positions, fills, and tips to
Postgres. Recovery: cold start from Postgres.

**Demo:** Binary that processes fills, flushes to Postgres, crashes
(kill -9), restarts, loads state from Postgres, verifies positions
match pre-crash state (within 10ms bounded loss).

**Files:** `persist.rs`, `replay.rs`, migrations

### Phase 4: Replication + Failover

Main/replica pair with fill buffering, tip sync, promotion, and
advisory lock lease management.

**Demo:** Start main + replica. Feed fills. Kill main (kill -9).
Observe replica promotes, continues processing. Show no fill loss.
Start new replica for the promoted main.

**Files:** `replica.rs`, `lease.rs`

### Phase 5: Full System (all components)

Complete risk engine with all components integrated. Multi-symbol,
multi-user, with funding, mark price feed, persistence, replication.

**Demo:** Full system with N mocked matching engines, M users.
Dashboard showing: fills/sec, margin recalcs/sec, Postgres flush
latency, position count, funding rates. Inject price crash ->
observe liquidations.

**Files:** `main.rs`, all integrated

## Tests

Tests: see [TESTING-RISK.md](TESTING-RISK.md) for complete unit
tests, e2e tests, integration tests, smoke tests, benchmarks,
correctness invariants, and test data patterns across all five
implementation phases.
