# Risk Engine Architecture

One instance per user shard. Single-threaded.

## Data Flow

```
Gateway (CMP)    ME (CMP)    Mark (CMP)
+----------+    +---------+  +---------+
| OrderReq |    | Fills   |  | MarkPx  |
+----------+    | Done    |  +---------+
    |           | Cancel  |      |
    v           | Insert  |      v
+-------------------------------------------+
| Risk Shard                                |
|                                           |
| 1. Drain fills (highest priority)         |
| 2. Process liquidation orders             |
| 3. Drain new orders (margin check)        |
| 4. Drain mark price updates              |
| 5. Drain BBOs (index price)              |
| 6. Funding settlement (if due)           |
|                                           |
| State: accounts, positions, tips,         |
|        mark_prices, index_prices,         |
|        frozen_orders, insurance_funds     |
+---+----------+---+-----------------------+
    |          |   |
    v          v   v
Gateway     ME    Postgres
(reject)  (order) (write-behind)
```

## Priority Order

1. **Fills** -- highest priority, position updates
2. **Liquidation orders** -- check pending liquidations
3. **New orders** -- pre-trade margin check
4. **Mark prices** -- update mark for margin calc
5. **BBOs** -- update index prices
6. **Funding** -- periodic settlement

## Margin Model

Portfolio margin across all positions per user.
Pre-trade check: simulate new position, verify
equity > initial margin requirement.

## Frozen Margin

Each accepted order freezes margin in a per-order
tracking map `FxHashMap<u128, (user_id, amount)>`.
Released on OrderDone or OrderCancelled via
`release_frozen_for_order()`.

## Liquidation

- Triggered when equity < maintenance margin
- LiquidationEngine queues (user, symbol) pairs
- Emits IOC reduce-only orders at mark price + slippage
- Multi-round with increasing slippage
- Socialized loss to insurance fund if unfillable

## Persistence

- Write-behind to Postgres via SPSC ring
- 10ms batched flush
- Positions, accounts, fills, tips, funding, liquidation
- Advisory lock: one writer per shard

## Recovery

1. Acquire advisory lock
2. Load positions + tips from Postgres
3. DXS replay from each ME: tips[symbol] + 1
4. Process replay fills (same code path as live)
5. On CaughtUp: connect gateway, go live

## Specs

- [specs/v1/RISK.md](../specs/v1/RISK.md)
- [specs/v1/LIQUIDATOR.md](../specs/v1/LIQUIDATOR.md)
- [specs/v1/DATABASE.md](../specs/v1/DATABASE.md)
